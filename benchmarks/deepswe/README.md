# DeepSWE A/B benchmark

This benchmark runs the same Codex model against the same deterministic DeepSWE task sample twice: plain Codex and Codex initialized with a pinned ForgeGuard release. Pier's held-out verifier remains the source of truth.

## Run

Prerequisites: Docker, Python 3.10+, `uv`, and a local [DeepSWE](https://github.com/datacurve-ai/deep-swe) checkout. The runner executes Pier `0.3.1` through `uvx` and pins Codex CLI `0.149.0`.

```sh
git clone https://github.com/datacurve-ai/deep-swe ../deep-swe
git -C ../deep-swe checkout 435ee89ec2f2e2289f33b0da4f992f0b7b7266b9
printf 'OPENAI_API_KEY=%s\n' "$OPENAI_API_KEY" > .env.deepswe
python3 benchmarks/deepswe/benchmark.py run \
  --tasks ../deep-swe/tasks \
  --model openai/gpt-5.5 \
  --n-tasks 10 \
  --seed 0 \
  --env-file .env.deepswe
```

The generated Pier config records Pier `0.3.1`, Codex CLI `0.149.0`, DeepSWE commit `435ee89ec2f2e2289f33b0da4f992f0b7b7266b9`, the model, deterministic sample, concurrency, and ForgeGuard version. The runner rejects another DeepSWE revision. The treatment installs ForgeGuard from its tagged installer, verifies its release checksum, and runs `forgeguard init --agent codex` before Codex starts. Generated policy files are excluded from the candidate patch, and any existing `.gitignore` is restored after initialization. Both conditions use one attempt per task.

The run writes Pier's raw evidence plus `jobs/deepswe/forgeguard-ab.summary.json`. Rebuild a summary without rerunning paid trials:

```sh
python3 benchmarks/deepswe/benchmark.py summarize jobs/deepswe/forgeguard-ab
```

The summary reports each condition's verifier rewards, errors, token use, cost, steps, and elapsed time. Paired wins/losses use only the verifier's numeric `reward`; missing evidence stays `null` and is excluded rather than inferred.

Start with 10 tasks to validate the setup. Use all 113 only after reviewing trajectories and confirming the treatment installed successfully. Do not tune ForgeGuard against held-out verifier files or select only favorable tasks.
