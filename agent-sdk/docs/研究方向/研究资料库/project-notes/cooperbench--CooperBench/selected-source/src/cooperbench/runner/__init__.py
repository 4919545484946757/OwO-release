"""Runner package - benchmark task execution."""

from cooperbench.runner.core import run
from cooperbench.runner.tasks import discover_tasks

__all__ = ["run", "discover_tasks"]
