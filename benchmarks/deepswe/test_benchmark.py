import argparse
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import benchmark


def result(task, agent, reward, tokens, cost, steps, seconds=10):
    return {
        "task_name": task,
        "agent_info": {"name": agent},
        "verifier_result": {"rewards": {"reward": reward, "tests": reward}},
        "agent_result": {
            "n_input_tokens": tokens,
            "n_output_tokens": tokens // 2,
            "cost_usd": cost,
            "n_agent_steps": steps,
        },
        "started_at": "2026-08-23T00:00:00+00:00",
        "finished_at": f"2026-08-23T00:00:{seconds:02d}+00:00",
    }


class BenchmarkTest(unittest.TestCase):
    def test_release_tag_rejects_shell_metacharacters(self):
        self.assertEqual(
            benchmark.validated_release_tag("v0.14.0-rc.1"), "v0.14.0-rc.1"
        )
        with self.assertRaises(ValueError):
            benchmark.validated_release_tag("v0.14.0; touch /tmp/injected")

    def test_job_name_rejects_output_path_traversal(self):
        self.assertEqual(benchmark.safe_job_name("forgeguard-ab.1"), "forgeguard-ab.1")
        with self.assertRaises(argparse.ArgumentTypeError):
            benchmark.safe_job_name("../outside")

    def test_run_rejects_another_deepswe_revision_before_writing_output(self):
        with tempfile.TemporaryDirectory() as directory:

            class Args:
                tasks = Path(directory)

            with patch.object(
                benchmark.subprocess,
                "run",
                return_value=SimpleNamespace(stdout="different-commit\n"),
            ):
                with self.assertRaisesRegex(SystemExit, "DeepSWE checkout must be"):
                    benchmark.run(Args())

            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_paired_summary_preserves_missing_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            job = Path(directory)
            fixtures = [
                result("a", benchmark.CONTROL, 0, 100, 1.0, 2),
                result("a", benchmark.TREATMENT, 1, 120, 1.2, 3),
                result("b", benchmark.CONTROL, 1, 200, 2.0, 4),
                result("b", benchmark.TREATMENT, None, 220, 2.2, 5),
            ]
            fixtures[-1]["verifier_result"] = None
            for index, fixture in enumerate(fixtures):
                trial = job / str(index)
                trial.mkdir()
                (trial / "result.json").write_text(json.dumps(fixture))

            summary = benchmark.make_summary(job)

        self.assertEqual(summary["paired_reward"]["matched_tasks"], 2)
        self.assertEqual(summary["paired_reward"]["scored_pairs"], 1)
        self.assertEqual(summary["paired_reward"]["treatment_wins"], 1)
        self.assertEqual(summary["paired_reward"]["mean_delta"], 1)
        self.assertEqual(
            summary["conditions"][benchmark.CONTROL]["totals"]["input_tokens"], 300
        )
        self.assertEqual(
            summary["conditions"][benchmark.TREATMENT]["mean_rewards"]["tests"], 1
        )

    def test_config_pairs_identical_dataset_and_model(self):
        class Args:
            model = "openai/test"
            reasoning_effort = "high"
            forgeguard_version = "v0.14.0"
            job_name = "test"
            jobs_dir = Path("jobs")
            tasks = Path("tasks")
            n_tasks = 7
            seed = 42
            concurrency = 2

        config = benchmark.build_config(Args())

        self.assertEqual(
            [agent["model_name"] for agent in config["agents"]], ["openai/test"] * 2
        )
        self.assertEqual(config["datasets"][0]["n_tasks"], 7)
        self.assertEqual(config["datasets"][0]["sample_seed"], 42)
        self.assertEqual(config["n_attempts"], 1)
        self.assertEqual(
            [agent["kwargs"]["version"] for agent in config["agents"]],
            [benchmark.CODEX_VERSION] * 2,
        )
        self.assertEqual(
            config["environment"]["env"]["FORGEGUARD_BENCHMARK_DEEPSWE_COMMIT"],
            benchmark.DEEPSWE_COMMIT,
        )


if __name__ == "__main__":
    unittest.main()
