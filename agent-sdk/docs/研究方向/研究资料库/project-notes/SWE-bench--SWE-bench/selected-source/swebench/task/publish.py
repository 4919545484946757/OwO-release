"""Compiling a task repo into the things people consume: a dataset, and images.

Both directions read the tree and nothing else, so what gets published is always
what the repo says. Every entry point checks the repo first: publishing from a
malformed tree is how a broken instance reaches a leaderboard.
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from swebench.task.checks import check_task_repo, errors
from swebench.task.repo import published_tasks, select_tasks

# `split` says which parquet a task belongs in; it is not a dataset column
# these route a task to its parquet; they are not dataset columns
INTERNAL_KEYS = ("split", "datasets")

# the eval card a dataset publishes alongside its rows, one per dataset:
# eval/SWE-bench_Lite.yaml
EVAL_FILE = "eval.yaml"
EVAL_SUBDIR = "eval"


class CheckFailed(RuntimeError):
    """The repo is not well formed, so nothing was published."""


def guard(repo_path: str | Path) -> None:
    """Refuse to go further if the repo has errors."""
    blocking = errors(check_task_repo(repo_path))
    if blocking:
        listed = "\n".join(f"  {p}" for p in blocking[:20])
        more = f"\n  ... and {len(blocking) - 20} more" if len(blocking) > 20 else ""
        raise CheckFailed(f"{len(blocking)} problem(s) in {repo_path}:\n{listed}{more}")


def _rows_for_split(instances: list[dict]) -> list[dict]:
    return [
        {k: v for k, v in inst.items() if k not in INTERNAL_KEYS} for inst in instances
    ]


def _align_columns(by_split: dict[str, list[dict]]) -> dict[str, list[dict]]:
    """Give every row in a dataset the same columns, null where a task has none.

    Optional metadata like `difficulty` is only on some tasks, but a dataset has
    one schema across its splits: a column present in test and absent in dev is
    rejected by the hub. Filled empty rather than null, so the column keeps the
    type it has on the tasks that do carry it.
    """
    empty = {str: "", list: [], dict: {}}
    columns = {}
    for rows in by_split.values():
        for row in rows:
            for column, value in row.items():
                columns.setdefault(column, empty.get(type(value)))
    return {
        split: [{**columns, **row} for row in rows] for split, rows in by_split.items()
    }


def compile_datasets(
    repo_path: str | Path, datasets: list[str] | None = None
) -> dict[str, dict[str, list[dict]]]:
    """What this repo publishes: rows per dataset, per split."""
    guard(repo_path)
    return {
        dataset: _align_columns(
            {split: _rows_for_split(rows) for split, rows in by_split.items()}
        )
        for dataset, by_split in published_tasks(repo_path, datasets).items()
    }


def _write_split(frame, path: Path) -> None:
    """Write one split, typing empty asset lists as strings.

    Every list in a column can be empty in one split and not in another -- only one
    multimodal instance has patch assets, and it is deprecated -- and arrow then types
    the column `list<null>` here and `list<string>` elsewhere, which makes the two
    splits disagree and fails a schema check on the hub.
    """
    import pyarrow as pa
    import pyarrow.parquet as pq

    table = pa.Table.from_pandas(frame, preserve_index=False)
    assets = "image_assets"
    if assets in table.schema.names:
        field = table.schema.field(assets)
        if pa.types.is_struct(field.type):
            fixed = pa.struct(
                [
                    pa.field(f.name, pa.list_(pa.string()))
                    if pa.types.is_list(f.type) and pa.types.is_null(f.type.value_type)
                    else f
                    for f in field.type
                ]
            )
            if fixed != field.type:
                i = table.schema.get_field_index(assets)
                table = table.set_column(
                    i, pa.field(assets, fixed), table.column(assets).cast(fixed)
                )
    pq.write_table(table, path)


def write_parquets(
    repo_path: str | Path, out_dir: str | Path, datasets: list[str] | None = None
) -> dict[str, Path]:
    """Write one parquet per split, per dataset, as `load_dataset` expects."""
    import pandas as pd

    written = {}
    for dataset, by_split in compile_datasets(repo_path, datasets).items():
        out = Path(out_dir) / dataset.split("/")[-1]
        out.mkdir(parents=True, exist_ok=True)
        for split, rows in by_split.items():
            path = out / f"{split}.parquet"
            _write_split(pd.DataFrame(rows), path)
            written[f"{dataset}/{split}"] = path
        card = eval_config(repo_path, dataset)
        if card:
            (out / EVAL_FILE).write_text(card.read_text())
            written[f"{dataset}/{EVAL_FILE}"] = out / EVAL_FILE
    return written


def eval_config(repo_path: str | Path, dataset: str) -> Path | None:
    """The eval card this dataset publishes, if the repo has one.

    One file per dataset, named for the dataset it belongs to, because Lite and
    Verified are not the same benchmark to a leaderboard even though one tree
    produces both.
    """
    path = Path(repo_path) / EVAL_SUBDIR / f"{dataset.split('/')[-1]}.yaml"
    return path if path.is_file() else None


def diff_against_hub(
    repo_path: str | Path, datasets: list[str] | None = None
) -> dict[str, dict]:
    """How the tree differs from what is published, per split.

    Reported per column rather than per row: "47 instances differ in eval_script"
    is actionable, a list of 47 ids is not.
    """
    from datasets import load_dataset

    report = {}
    for dataset, by_split in compile_datasets(repo_path, datasets).items():
        for split, rows in by_split.items():
            tree = {r["instance_id"]: r for r in rows}
            key = f"{dataset}/{split}"
            try:
                live = {r["instance_id"]: r for r in load_dataset(dataset, split=split)}
            except Exception as e:  # noqa: BLE001 - datasets raises many types when a dataset or split is absent
                report[key] = {"error": f"{type(e).__name__}: {e}"}
                continue
            added = sorted(set(tree) - set(live))
            removed = sorted(set(live) - set(tree))
            changed: dict[str, int] = {}
            for iid in set(tree) & set(live):
                for column in tree[iid]:
                    if column in live[iid] and tree[iid][column] != live[iid][column]:
                        changed[column] = changed.get(column, 0) + 1
            report[key] = {"added": added, "removed": removed, "changed": changed}
    return report


def push_dataset(
    repo_path: str | Path, datasets: list[str] | None = None, dry_run: bool = False
) -> dict:
    """Overwrite the datasets named in sweb.yaml with the tree."""
    compiled = compile_datasets(repo_path, datasets)
    cards = {name: eval_config(repo_path, name) for name in compiled}
    plan = {
        "datasets": {
            name: {split: len(rows) for split, rows in by_split.items()}
            for name, by_split in compiled.items()
        },
        "eval_config": {name: str(path) for name, path in cards.items() if path},
        "diff": diff_against_hub(repo_path, datasets),
    }
    if dry_run:
        return plan

    from datasets import Dataset, DatasetDict
    from huggingface_hub import HfApi

    for name, by_split in compiled.items():
        # every split in one commit: pushing them one at a time leaves the splits
        # that have not been pushed yet disagreeing with the ones that have, which
        # the hub rejects
        DatasetDict(
            {split: Dataset.from_list(rows) for split, rows in by_split.items()}
        ).push_to_hub(name)
        # after the rows, so the card never lands on a dataset that failed to push
        if cards[name]:
            HfApi().upload_file(
                path_or_fileobj=cards[name],
                path_in_repo=EVAL_FILE,
                repo_id=name,
                repo_type="dataset",
            )
    return plan


def _images_for(
    repo_path: str | Path,
    instance_ids: list[str] | None,
    datasets: list[str] | None = None,
) -> dict[str, str]:
    return {
        i["instance_id"]: i["image"]
        for i in select_tasks(repo_path, instance_ids, datasets)
    }


def push_images(
    repo_path: str | Path,
    instance_ids: list[str] | None = None,
    dry_run: bool = False,
    datasets: list[str] | None = None,
) -> dict:
    """Push each task's image under exactly the name its task.yaml declares.

    The dataset tells the harness which image to pull, so publishing any other
    name produces a dataset that cannot be run.
    """
    guard(repo_path)
    images = _images_for(repo_path, instance_ids, datasets)

    import docker

    client = docker.from_env()
    local = {tag for image in client.images.list() for tag in (image.tags or [])}

    missing = sorted(name for name in images.values() if name not in local)
    pushable = sorted(name for name in images.values() if name in local)
    plan = {
        "to_push": pushable,
        "not_built": missing,
        "commands": [f"docker push {name}" for name in pushable],
    }
    if dry_run:
        return plan

    pushed, failed = [], {}
    for name in pushable:
        result = subprocess.run(
            ["docker", "push", name], capture_output=True, text=True, check=False
        )
        if result.returncode == 0:
            pushed.append(name)
        else:
            failed[name] = (result.stderr.strip().splitlines() or ["push failed"])[-1]
    plan["pushed"] = pushed
    plan["failed"] = failed
    return plan


def summarize(plan: dict) -> str:
    return json.dumps(plan, indent=2, default=str)
