#!/usr/bin/env python3
"""Run and summarize a paired DeepSWE evaluation with no extra dependencies."""

import argparse
import json
import os
import re
from datetime import datetime
from pathlib import Path
import subprocess
import sys

from release_tag import validated_release_tag

CONTROL = "codex"
TREATMENT = "codex-forgeguard"
PIER_REQUIREMENT = "datacurve-pier==0.3.1"
CODEX_VERSION = "0.149.0"
DEEPSWE_COMMIT = "435ee89ec2f2e2289f33b0da4f992f0b7b7266b9"


def positive_int(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return number


def safe_job_name(value):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value):
        raise argparse.ArgumentTypeError(
            "must contain only letters, digits, '.', '_', or '-' and start with a letter or digit"
        )
    return value


def build_config(args):
    common = {
        "model_name": args.model,
        "kwargs": {
            "reasoning_effort": args.reasoning_effort,
            "version": CODEX_VERSION,
        },
    }
    return {
        "job_name": args.job_name,
        "jobs_dir": str(args.jobs_dir.resolve()),
        "n_attempts": 1,
        "n_concurrent_trials": args.concurrency,
        "environment": {
            "env": {
                "FORGEGUARD_BENCHMARK_PIER": PIER_REQUIREMENT,
                "FORGEGUARD_BENCHMARK_CODEX": CODEX_VERSION,
                "FORGEGUARD_BENCHMARK_DEEPSWE_COMMIT": DEEPSWE_COMMIT,
                "FORGEGUARD_BENCHMARK_FORGEGUARD": args.forgeguard_version,
            }
        },
        "agents": [
            {"name": CONTROL, **common},
            {
                "import_path": "forgeguard_codex:ForgeGuardCodex",
                **common,
                "kwargs": {
                    **common["kwargs"],
                    "forgeguard_version": args.forgeguard_version,
                },
            },
        ],
        "datasets": [
            {
                "path": str(args.tasks.resolve()),
                "n_tasks": args.n_tasks,
                "sample_seed": args.seed,
            }
        ],
    }


def run(args):
    if not args.tasks.is_dir():
        raise SystemExit(f"DeepSWE tasks directory not found: {args.tasks}")
    revision = subprocess.run(
        ["git", "-C", str(args.tasks), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != DEEPSWE_COMMIT:
        raise SystemExit(
            f"DeepSWE checkout must be {DEEPSWE_COMMIT}, found {revision or 'unknown'}"
        )
    args.jobs_dir.mkdir(parents=True, exist_ok=True)
    config_path = args.jobs_dir / f"{args.job_name}.config.json"
    config_path.write_text(json.dumps(build_config(args), indent=2) + "\n")
    print(f"Wrote {config_path}")
    if args.dry_run:
        return

    command = [
        "uvx",
        "--from",
        PIER_REQUIREMENT,
        "pier",
        "run",
        "--config",
        str(config_path),
        "--yes",
    ]
    if args.env_file:
        command.extend(["--env-file", str(args.env_file.resolve())])
    env = os.environ.copy()
    module_dir = str(Path(__file__).resolve().parent)
    env["PYTHONPATH"] = os.pathsep.join(
        value for value in (module_dir, env.get("PYTHONPATH")) if value
    )
    subprocess.run(command, check=True, env=env)
    summarize(
        args.jobs_dir / args.job_name, args.jobs_dir / f"{args.job_name}.summary.json"
    )


def seconds_between(start, finish):
    if not start or not finish:
        return None
    return (
        datetime.fromisoformat(finish) - datetime.fromisoformat(start)
    ).total_seconds()


def numeric_sum(values):
    present = [value for value in values if isinstance(value, (int, float))]
    return sum(present) if present else None


def trial_metrics(result):
    context = result.get("agent_result") or {}
    rewards = (result.get("verifier_result") or {}).get("rewards") or {}
    return {
        "task": result["task_name"],
        "reward": rewards.get("reward"),
        "rewards": rewards,
        "input_tokens": context.get("n_input_tokens"),
        "cache_tokens": context.get("n_cache_tokens"),
        "output_tokens": context.get("n_output_tokens"),
        "cost_usd": context.get("cost_usd"),
        "steps": result.get("n_agent_steps", context.get("n_agent_steps")),
        "elapsed_seconds": seconds_between(
            result.get("started_at"), result.get("finished_at")
        ),
        "error": (result.get("exception_info") or {}).get("exception_type"),
    }


def load_trials(job_dir):
    trials = {CONTROL: {}, TREATMENT: {}}
    for path in job_dir.rglob("result.json"):
        result = json.loads(path.read_text())
        agent = (result.get("agent_info") or {}).get("name")
        if agent not in trials:
            continue
        task = result.get("task_name")
        if task in trials[agent]:
            raise ValueError(
                f"duplicate result for {agent}/{task}; use one attempt per task"
            )
        trials[agent][task] = trial_metrics(result)
    return trials


def aggregate(trials):
    reward_names = sorted(
        {name for trial in trials.values() for name in trial["rewards"]}
    )
    return {
        "trials": len(trials),
        "errors": sum(trial["error"] is not None for trial in trials.values()),
        "mean_rewards": {
            name: (
                # forgeguard: allow FG-ALG-001 -- at most 113 tasks and a small verifier reward map; pre-index if either grows
                numeric_sum([trial["rewards"].get(name) for trial in trials.values()])
                # forgeguard: allow FG-ALG-001 -- at most 113 tasks and a small verifier reward map; pre-index if either grows
                / sum(
                    isinstance(trial["rewards"].get(name), (int, float))
                    for trial in trials.values()
                )
            )
            for name in reward_names
            # forgeguard: allow FG-ALG-001 -- at most 113 tasks and a small verifier reward map; pre-index if either grows
            if any(
                isinstance(trial["rewards"].get(name), (int, float))
                for trial in trials.values()
            )
        },
        "totals": {
            # forgeguard: allow FG-ALG-001 -- six fixed fields over at most 113 tasks; aggregate in one pass if the schema grows
            field: numeric_sum([trial[field] for trial in trials.values()])
            for field in (
                "input_tokens",
                "cache_tokens",
                "output_tokens",
                "cost_usd",
                "steps",
                "elapsed_seconds",
            )
        },
    }


def make_summary(job_dir):
    trials = load_trials(job_dir)
    matched = sorted(set(trials[CONTROL]) & set(trials[TREATMENT]))
    reward_pairs = [
        (trials[CONTROL][task]["reward"], trials[TREATMENT][task]["reward"])
        for task in matched
    ]
    reward_pairs = [
        pair
        for pair in reward_pairs
        # forgeguard: allow FG-ALG-001 -- each pair always has exactly two values
        if all(isinstance(value, (int, float)) for value in pair)
    ]
    return {
        "job_dir": str(job_dir.resolve()),
        "conditions": {name: aggregate(values) for name, values in trials.items()},
        "paired_reward": {
            "matched_tasks": len(matched),
            "scored_pairs": len(reward_pairs),
            "treatment_wins": sum(
                treatment > control for control, treatment in reward_pairs
            ),
            "control_wins": sum(
                control > treatment for control, treatment in reward_pairs
            ),
            "ties": sum(control == treatment for control, treatment in reward_pairs),
            "mean_delta": (
                sum(treatment - control for control, treatment in reward_pairs)
                / len(reward_pairs)
                if reward_pairs
                else None
            ),
        },
        "unmatched_tasks": {
            CONTROL: sorted(set(trials[CONTROL]) - set(trials[TREATMENT])),
            TREATMENT: sorted(set(trials[TREATMENT]) - set(trials[CONTROL])),
        },
    }


def summarize(job_dir, output=None):
    if not job_dir.is_dir():
        raise SystemExit(f"Pier job directory not found: {job_dir}")
    summary = make_summary(job_dir)
    rendered = json.dumps(summary, indent=2) + "\n"
    if output:
        output.write_text(rendered)
        print(f"Wrote {output}")
    else:
        print(rendered, end="")


def parser():
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    run_parser = commands.add_parser("run", help="run the paired benchmark")
    run_parser.add_argument("--tasks", type=Path, required=True)
    run_parser.add_argument("--model", required=True)
    run_parser.add_argument("--n-tasks", type=positive_int, default=10)
    run_parser.add_argument("--seed", type=int, default=0)
    run_parser.add_argument("--concurrency", type=positive_int, default=4)
    run_parser.add_argument("--reasoning-effort", default="high")
    run_parser.add_argument(
        "--forgeguard-version", type=validated_release_tag, default="v0.14.0"
    )
    run_parser.add_argument("--jobs-dir", type=Path, default=Path("jobs/deepswe"))
    run_parser.add_argument("--job-name", type=safe_job_name, default="forgeguard-ab")
    run_parser.add_argument("--env-file", type=Path)
    run_parser.add_argument("--dry-run", action="store_true")
    run_parser.set_defaults(function=run)

    summary_parser = commands.add_parser(
        "summarize", help="summarize an existing Pier job"
    )
    summary_parser.add_argument("job_dir", type=Path)
    summary_parser.add_argument("--output", type=Path)
    summary_parser.set_defaults(
        function=lambda args: summarize(args.job_dir, args.output)
    )
    return root


if __name__ == "__main__":
    arguments = parser().parse_args()
    try:
        arguments.function(arguments)
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
