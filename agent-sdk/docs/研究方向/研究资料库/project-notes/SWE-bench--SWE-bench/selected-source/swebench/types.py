"""
Type definitions for swebench.
"""

from typing import TypedDict
from dataclasses import dataclass, field


class SWEbenchInstance(TypedDict):
    repo: str
    instance_id: str
    base_commit: str
    patch: str
    test_patch: str
    problem_statement: str
    hints_text: str
    created_at: str
    version: str
    FAIL_TO_PASS: str
    PASS_TO_PASS: str
    environment_setup_commit: str


@dataclass
class TestSpec:
    """
    A dataclass that represents a test specification for evaluation of a single instance of SWE-bench.
    Assumes images are already built and available.
    """

    instance_id: str
    image: str
    eval_script_list: list[str]
    repo: str
    version: str
    FAIL_TO_PASS: list[str]
    PASS_TO_PASS: list[str]
    log_parser: str = ""
    eval_type: str = ""
    # Multimodal binary assets (e.g. expected.png baselines) that a text patch
    # cannot carry; {"test_patch": [{"path": ..., "url": ...}], ...}
    image_assets: dict = field(default_factory=dict)

    @property
    def eval_script(self):
        return (
            "\n".join(["#!/bin/bash", "set -uxo pipefail"] + self.eval_script_list)
            + "\n"
        )
