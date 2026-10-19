# PASC — Peer-Aware Action-Space Coordination

Replication package for *Coordinating Agents on a Shared Git Workspace: An
Empirical Study of Action-Space Observation for Agentic Software Engineering*,
published at the 20th International Symposium on Empirical Software
Engineering and Measurement (ESEM 2026).

[![Paper](https://img.shields.io/badge/paper-LIPIcs.ESEM.2026.18-blue)](https://doi.org/10.4230/LIPIcs.ESEM.2026.18)
[![DOI](https://zenodo.org/badge/DOI/10.5281/zenodo.20276750.svg)](https://doi.org/10.5281/zenodo.20276750)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**Authors:** Hasen Özaytürk and Feza Buzluca, Istanbul Technical University
**Paper:** [doi:10.4230/LIPIcs.ESEM.2026.18](https://doi.org/10.4230/LIPIcs.ESEM.2026.18)
**Archived package:** [doi:10.5281/zenodo.20276750](https://doi.org/10.5281/zenodo.20276750)

PASC is a multi-agent coordination layer built on top of `mini-swe-agent`.
N agents share one Docker container per SWE-Bench Pro instance and coordinate
via auto-committed git history + a structured peer-activity feed. **This
README is a usage guide; the paper covers the mechanism and evaluation.**


## Repository layout

```
.
├── pasc/                                 # PASC coordination layer
│   ├── shared.py                         # Common dataclasses + helpers
│   ├── bootstrap.py                      # mini-swe-agent path setup
│   └── gitops/                           # Orchestrator + components
│       ├── orchestrator.py               # GitOpsOrchestrator entrypoint
│       ├── agent.py                      # GitOpsAgent (DefaultAgent subclass)
│       ├── workspace.py                  # SharedWorkspace + FederatedWorkspaces
│       ├── run_state.py                  # AgentStatusBoard + DigestBoard + PatchRegistry
│       ├── peer_activity.py              # Builds the <peer_activity> block
│       ├── output_digest.py              # One-line digest extractor (6 patterns)
│       ├── commit_tracker.py             # Thread-safe commit hook + last-seen bookkeeping
│       ├── git_ops.py                    # docker exec ... git ... wrapper
│       └── shared_environment.py         # DockerEnvironment composition wrapper
│
├── mini-swe-agent/                       # Vendored upstream (MIT)
│
├── FINAL_results/                        # Paper-final evaluation outputs
│   ├── pasc_{minimax_m27,deepseek_v4pro}/         # PASC multi-agent, primary run
│   ├── silent_{minimax_m27,deepseek_v4pro}/       # Ablation: peer feeds off
│   ├── single_{minimax_m27,deepseek_v4pro}/       # Baseline: single-agent
│   ├── federated_{minimax_m27,deepseek_v4pro}/    # Federated workspace mode
│   ├── *_run2/, *_run3/                  # Replication runs (3× each config)
│   ├── peer_3_5_mm_ablations/            # N=3 and N=5 sweeps (MiniMax)
│   ├── figure_2/                         # Curated example trajectories used in the paper
│   ├── ablations/                        # Per-feature ablation summary JSONs
│   ├── dual_container_summary.json       # Aggregate: PASC vs silent vs single
│   ├── replication_per_run.json          # Per-replication pass-rate summary
│   └── token_summary.json                # Aggregate token-usage summary
│
├── FINAL_analysis_scripts/               # Paper tables + figures producers
│   ├── README.md                         # Per-script usage
│   ├── significance.py                   # OUT-OF-THE-BOX: McNemar + Fisher (uses shipped JSON)
│   ├── advanced_stats.py                 # OUT-OF-THE-BOX: GLM, TOST, Cochran-Armitage, MH, power
│   ├── analyze_silent_vs_pasc.py         # Regenerates significance.py's input from raw runs/
│   ├── analyze_nagent.py                 # Regenerates advanced_stats.py's input from raw runs/
│   ├── destructive_thrash*.py            # Detector libraries used by the analyzers
│   ├── silent_vs_pasc_thrash.json        # Precomputed input for significance.py
│   └── nagent_thrash.json                # Precomputed input for advanced_stats.py
│
├── run_batch_gitops.py                   # Multi-agent batch driver
├── run_gitops.py                         # Single-instance variant (smoke testing)
├── requirements.txt                      # Extra pip deps beyond mini-swe-agent (scipy, statsmodels)
├── README.md, LICENSE, CITATION.cff
└── THIRD_PARTY_LICENSES/                 # mini-swe-agent attribution + license text
```

Total: 405 files, 467 MB (most of it eval artifacts in `FINAL_results/`). Raw
per-instance trajectories (`runs/`) are NOT shipped (too large); see
`FINAL_analysis_scripts/README.md` for how to regenerate them.

Three prediction files exceed GitHub's 100 MB per-file limit and are stored
gzipped in this repository — `silent_minimax_m27_run3`,
`silent_minimax_m27_run2` and `silent_deepseek_v4pro_run2`. Restore them
before scoring:

```bash
gunzip FINAL_results/*/preds.json.gz
```

Everything the analysis scripts read (`eval_results.json`, `token_report.json`)
is uncompressed, so the paper's tables reproduce without this step.

DeepSeek-V4-Pro costs are reported at the provider's current price
($0.435/M input cache-miss, $0.87/M output) — the promotional rate in effect
during the runs, since made permanent. The earlier list price is retained per
configuration under `cost_at_former_list_price` in `token_report.json`.
MiniMax-M2.7 pricing is unchanged.


## Prerequisites

| Tool / asset | Why |
|---|---|
| Python 3.10+ | type-hint syntax (PEP 604, slots dataclasses) |
| Docker | sandboxed agent execution; image-per-instance pulled from `jefzda/sweap-images` |
| Hugging Face access | dataset `ScaleAI/SWE-bench_Pro` |
| Anthropic-API-compatible LLM | direct provider or LiteLLM proxy (see `mini-swe-agent/src/minisweagent/models/`) |

Resource expectations per single instance, n=2 agents: ~2 min wall clock,
~$0.05–0.50 in LLM cost depending on provider/model. Per-instance Docker
image is ~1 GB (cached after first pull).


## Install

```bash
# 1. vendored mini-swe-agent + its core dependencies
#    (litellm, datasets, typer, pyyaml, numpy, pandas, ...)
cd mini-swe-agent && pip install -e . && cd ..

# 2. additional Python deps (scipy + statsmodels, used by FINAL_analysis_scripts/advanced_stats.py)
pip install -r requirements.txt
```

This adds `mini-extra` (single-agent batch CLI) and `mini` to your PATH,
plus all Python deps PASC and the analysis scripts need.

Docker daemon must be running before launching any agent run; per-instance
images are auto-pulled from `jefzda/sweap-images:*` on first use (~1 GB
each, cached locally afterwards).


## Configure model access

Set the API key for whichever provider you target. The model class is
selected at the command line; the key your env exports must match.

```bash
# Anthropic direct
export ANTHROPIC_API_KEY=sk-ant-...

# LiteLLM proxy (any provider)
export OPENAI_API_KEY=...                # or PROVIDER-specific var
```


## Environment variables

| Variable | Values | Default | Meaning |
|---|---|---|---|
| `PASC_GITOPS_OBSERVATION_ROLE` | `user` / `tool` | `user` | Which message role carries the bash observation. Use `tool` for native-tool providers (DeepSeek-V4-Pro etc.); leave default for text-bash providers (Anthropic, MiniMax). |

## CLI flags (key ones)

| Flag | Values | Default | Meaning |
|---|---|---|---|
| `--workspace` | `shared` / `federated` | `shared` | Workspace strategy. **`shared`**: all agents commit to one tree, the team submission is `git diff base..HEAD`. **`federated`**: each agent gets a private clone of the base tree; one patch per agent, the first non-empty by agent-id order becomes the headline `final.patch`. See [§ Workspace modes](#workspace-modes) below. |
| `-n N` | int | `2` | Number of agents per instance (sharing one Docker container). |
| `-w P` | int | `2` | Parallel instances (each in its own container). |


## Smoke test (single instance)

```bash
echo "instance_qutebrowser__qutebrowser-0b621cb0ce2b54d3f93d8d41d8ff4257888a87e5-v2ef375ac784985212b1805e1d0431dc8f1b3c171" > /tmp/one.txt

python3 run_batch_gitops.py \
  --instances-file /tmp/one.txt \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -n 2 -o runs/smoke \
  --workspace shared \
  -m <model> --model-class <class> \
  -c swebench_xml_gitops \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true \
  -c agent.step_limit=250
```

Add `--workspace federated` to verify the federated mode end-to-end on the
same instance (the orchestrator will clone the base tree once per agent
inside the container).

Expected: `runs/smoke/preds_list.json` with one non-empty entry, plus the
per-run directory described in [§ Run output layout](#run-output-layout).


## Reproducing the paper configurations

The configurations and per-instance predictions in the paper are encoded
in `FINAL_results/<config>/preds.json`. The commands below regenerate
them from scratch with `-w 1` (one instance at a time — bump `-w` to
parallelize across containers). All commands use:

- `instance_list.txt` — newline-separated `instance_id`s (use the keys of
  any `FINAL_results/<config>/eval_results.json` as the canonical 266-Python
  list). Used by `run_batch_gitops.py`; the single-agent baseline below
  selects the same instances with `--filter` instead.
- `<model> / <class>` — e.g. `MiniMax-M2.7-highspeed` + `minimax`,
  `deepseek/deepseek-v4-pro` + `litellm`, `claude-sonnet-4-5` + `anthropic_sdk`.
- `temperature=0.0` for deterministic runs (matches the paper).
- `drop_params=true` is litellm-only; safe to drop for the Anthropic SDK.
- `agent.step_limit=250` — the paper's per-agent step budget.

```bash
# ===== PASC — N=2 shared workspace (paper headline) =====
python3 run_batch_gitops.py \
  --instances-file <project_path>/instance_list.txt \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -n 2 -o runs/pasc_n2 \
  --workspace shared \
  -m <model> --model-class <class> \
  -c swebench_xml_gitops \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true \
  -c agent.step_limit=250
```

```bash
# ===== Silent ablation — N=2, peer feeds disabled =====
# Same orchestrator as PASC, swap the YAML overlay.
python3 run_batch_gitops.py \
  --instances-file <project_path>/instance_list.txt \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -n 2 -o runs/silent_n2 \
  --workspace shared \
  -m <model> --model-class <class> \
  -c swebench_xml_gitops_silent \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true \
  -c agent.step_limit=250
```

```bash
# ===== Federated workspace — N=2, one private tree per agent =====
python3 run_batch_gitops.py \
  --instances-file <project_path>/instance_list.txt \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -n 2 -o runs/federated_n2 \
  --workspace federated \
  -m <model> --model-class <class> \
  -c swebench_xml_gitops_federated \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true \
  -c agent.step_limit=250
```

```bash
# ===== PASC — N>2 agents (e.g. N=5) =====
# The npeer YAML rewrites the system prompt for >2 peers
# ("you and your peers" instead of "you and your peer").
python3 run_batch_gitops.py \
  --instances-file <project_path>/instance_list.txt \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -n 5 -o runs/pasc_n5 \
  --workspace shared \
  -m <model> --model-class <class> \
  -c swebench_xml_gitops_npeer \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true \
  -c agent.step_limit=250
```

```bash
# ===== Single-agent baseline (no PASC overlay) =====
# Uses mini-swe-agent's own batch CLI. NOTE: this runner selects instances by
# regex over instance_id (--filter), not from a file — the 266-instance Python
# subset is exactly the ansible / internetarchive / qutebrowser instances of the
# Pro test split. It writes preds.json only (a dict keyed by instance_id whose
# entries carry `model_patch`); convert it to the list format below.
mini-extra swebench \
  --filter 'instance_(ansible|internetarchive|qutebrowser)__' \
  --subset ScaleAI/SWE-bench_Pro --split test \
  --environment-class docker \
  -w 1 -o runs/single \
  -m <model> --model-class <class> \
  -c swebench \
  -c 'environment.run_args=["--rm","--platform","linux/amd64","--entrypoint","/usr/bin/env"]' \
  -c model.model_kwargs.temperature=0.0 \
  -c model.model_kwargs.drop_params=true

# The evaluator takes the SWE-bench list format; derive it from preds.json:
python3 -c "import json; d=json.load(open('runs/single/preds.json')); \
json.dump(list(d.values()), open('runs/single/preds_list.json','w'), indent=2)"
```

Provider-specific notes:

- **MiniMax via Anthropic-compatible endpoint**: add
  `-c 'model.base_url=https://api.minimax.io/anthropic'`.
- **DeepSeek-V4-Pro and other native-tool providers**: prepend
  `PASC_GITOPS_OBSERVATION_ROLE=tool` so bash observations are wrapped
  as a tool-call result rather than a user message.
- **Cost tracking**: append `-c model.cost_tracking=ignore_errors` to
  silence per-call cost-lookup warnings for providers without litellm
  pricing tables.

Common flags:
- `-w P` parallel **instances** (each in its own container).
- `-n N` parallel **agents per instance** (all sharing one container; PASC only).
- `-o DIR` per-run output directory under `runs/`.
- `-m MODEL --model-class CLASS` selects the LLM endpoint.
- `-c YAML` overlay configs; multiple `-c` flags merge in order.
- `-c key=value` inline override (e.g. `-c agent.step_limit=250`).


## Run output layout

```
runs/<output>/
└── <instance_id>/
    └── pasc_gitops_<YYYYMMDD>_<HHMMSS>_<nanos>/
        ├── orchestrator.log              # per-step events (commits, peer_activity_injected,
        │                                  digest_published, stuck_signaled, agent_complete,
        │                                  federated_tree_cloned in federated mode, ...)
        ├── merge_report.json             # workspace_mode, base_commit, head_commits (per-agent),
        │                                  headline_agent_id, patches (per-agent bytes + path),
        │                                  commit_history, agent_results, seed_slots
        ├── final.patch                   # Headline patch. Shared mode: the team `git diff base..HEAD`.
        │                                  Federated mode: the first non-empty per-agent patch.
        ├── patches/
        │   ├── agent_0.diff              # Per-agent diff. Shared mode: identical for every agent
        │   ├── agent_1.diff              #   (= team patch). Federated mode: distinct per agent.
        │   └── ...
        ├── run_state.json                # RunState snapshot: agent_status + per-agent patch records
        ├── agent_0/
        │   ├── trajectory.json           # full agent message history + extra (cost, response, ...)
        │   ├── raw.patch                 # agent's own submission via COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT
        │   │                                (kept for debugging; NOT used as the headline patch)
        │   └── result.json               # AgentPatchResult dict (exit_status, status, paths, ...)
        └── agent_1/, agent_2/, ...       # one directory per agent in the run
```

Top-level aggregates written after the whole batch:
```
runs/<output>/
├── preds_list.json                       # SWE-bench list format: [{instance_id, patch, model_name_or_path}]
└── preds.json                            # Same data keyed by instance_id (SWE-bench dict format)
```

Both files use the **headline** patch as `patch` / `model_patch`. To run a
SWE-bench evaluation against any individual agent's contribution in
federated mode, rebuild the preds files from `patches/agent_X.diff` —
the per-agent patches are saved verbatim alongside the headline.


## Evaluating predictions on SWE-bench Pro

The `preds_list.json` file produced by `run_batch_gitops.py` is in the
format the SWE-bench Pro evaluator (the `SWE-bench_Pro-os` companion to
the `ScaleAI/SWE-bench_Pro` dataset) expects. Scoring is a separate step
that runs each predicted patch against the benchmark's hidden test suite
inside a Docker sandbox.

```bash
# 1. Clone the SWE-bench Pro evaluator (one-time setup)
git clone https://github.com/scaleapi/SWE-bench_Pro-os.git
cd SWE-bench_Pro-os

# 2. Score one config's predictions
python3 swe_bench_pro_eval.py \
  --raw_sample_path swe_bench_pro_full.csv \
  --patch_path <path-to>/runs/<run_name>/preds_list.json \
  --output_dir eval_results_<run_name> \
  --scripts_dir run_scripts \
  --dockerhub_username jefzda \
  --use_local_docker \
  --num_workers 4
```

This produces `<output_dir>/eval_results.json` mapping each `instance_id`
to a `resolved` / `unresolved` verdict — the same shape as the
`FINAL_results/<config>/eval_results.json` files shipped here. Pass rate
is the share of `resolved` over total instances.

Federated runs emit per-agent prediction files (`preds_list_<agent_id>.json`)
alongside the headline `preds_list.json`. Each can be scored independently
to compare per-agent contributions.


## Workspace modes

`--workspace shared` (default): all N agents commit to one tree. The team's
deliverable is `git diff base..HEAD`. Every `patches/agent_X.diff` is identical
to `final.patch`. **This is the configuration the paper's headline numbers use.**

`--workspace federated`: each agent owns a private clone of the base tree.
Per-agent patches are written separately; `final.patch` is the first non-empty
one in agent-id order. Useful for ablating the shared-workspace assumption.

| | Shared | Federated |
|---|---|---|
| Number of trees | 1 | N |
| `final.patch` | team diff | first non-empty per-agent |
| `patches/agent_X.diff` | identical (= team patch) | distinct per agent |
| Agents can overwrite each other | yes | no |
| Default | yes | opt-in |


## Analysis scripts

`FINAL_analysis_scripts/significance.py` runs out-of-the-box against the
shipped `silent_vs_pasc_thrash.json`:

```bash
python3 FINAL_analysis_scripts/significance.py
# → McNemar exact + Fisher exact p-values for each (model, signal) pair
```

`FINAL_analysis_scripts/advanced_stats.py` runs against the shipped
`nagent_thrash.json`:

```bash
python3 FINAL_analysis_scripts/advanced_stats.py
# → logistic GLM, TOST equivalence (N=2 vs N=5), Cochran-Armitage trend,
#   Mantel-Haenszel stratified test, McNemar power calculation
```

`FINAL_analysis_scripts/analyze_silent_vs_pasc.py` and `analyze_nagent.py`
REGENERATE the intermediate JSONs from raw `runs/<config>/instance_*/pasc_gitops_*`
trajectories. **These traces are NOT shipped** (each is ~hundreds of MB);
to regenerate set `PASC_REPO_ROOT` to a tree containing the runs you
produced via `run_batch_gitops.py`:

```bash
PASC_REPO_ROOT=/path/to/my/runs_tree python3 FINAL_analysis_scripts/analyze_silent_vs_pasc.py
```


## License

Source code in this repository is released under the MIT license — see `LICENSE`.
The evaluation outputs in `FINAL_results/` and the documentation are released
under [CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/), matching the
archived Zenodo record
([10.5281/zenodo.20276750](https://doi.org/10.5281/zenodo.20276750)).

`mini-swe-agent/` is vendored from the upstream project under its own MIT
license — see `THIRD_PARTY_LICENSES/mini-swe-agent.LICENSE` and the
provenance `NOTICE.md`.


## Citation

If you use this code or data, please cite the paper:

```bibtex
@InProceedings{ozayturk_et_al:LIPIcs.ESEM.2026.18,
  author =	{\"{O}zayt\"{u}rk, Hasen and Buzluca, Feza},
  title =	{{Coordinating Agents on a Shared Git Workspace: An Empirical Study of Action-Space Observation for Agentic Software Engineering}},
  booktitle =	{20th International Symposium on Empirical Software Engineering and Measurement (ESEM 2026)},
  pages =	{18:1--18:20},
  series =	{Leibniz International Proceedings in Informatics (LIPIcs)},
  volume =	{394},
  editor =	{Feldt, Robert and Paasivaara, Maria and Mendez, Daniel and Wagner, Stefan and Mu\~{n}oz Bar\'{o}n, Marvin},
  publisher =	{Schloss Dagstuhl -- Leibniz-Zentrum f\"{u}r Informatik},
  address =	{Dagstuhl, Germany},
  year =	{2026},
  doi =		{10.4230/LIPIcs.ESEM.2026.18}
}
```

## Funding

This work was supported by Research Fund of the Istanbul Technical University.
Project Number: 48456.

## Contact

- Hasen Özaytürk — ozayturk22@itu.edu.tr ([ORCID](https://orcid.org/0009-0003-0392-4073))
- Feza Buzluca — buzluca@itu.edu.tr ([ORCID](https://orcid.org/0000-0001-9589-8754))

Issues and pull requests are welcome at
<https://github.com/hasenoz/PASC>.
