# agent-frame integration verification (2026-10-11)

Integrated existing agent-frame head 3dfc675dce50f03a4ef47cc4b20ee1231efa063f
(35 commits since the common base) with reliability repair 60a421ff0222d8e6f0bb671ab68dd14f6c6dab72
and desktop improvements 0ac35f7a647604f60433284dd2e556273006e0fe. Both histories are retained.

Three conflict resolutions preserve Anthropic response caps, request timeouts and
credential redaction; OpenAI custom full URLs and temperature during reasoning-only
retry; bounded stream frames and cancellation-safe circuit-breaker permits.
The reasoning-only retry test now verifies the same custom URL and temperature on
both attempts. No cloud model was called.

Verification after integration:
- Web + Electron: 780 passed, 0 failed.
- Rust server + CLI check: passed; existing CLI dead_code warning remains.
- Core + server Clippy with -D warnings: passed.
- Workspace fmt --check: passed.
- Gateway unit tests: 56 passed, 2 intentionally ignored cloud tests.
- Anthropic unit tests: 8 passed.
- Desktop focused contracts: 4 passed (also included in the route suite).
- Complete server route contracts: 51 passed after fixture isolation repair.

The first complete route run failed because DELETE /session/{id} consumed the shared
session fixture before subsequent route checks. DELETE now uses a disposable session;
missing-route assertions remain strict. The initial failure and successful retry logs
are both retained.

The previously modified builGoal document is preserved byte-for-byte and is excluded
from this merge commit. Other untracked local documents, profiles and test data remain
local. The desktop review ZIP remains the immutable pre-integration review snapshot;
these logs document validation of the combined source. No installer or GUI release
acceptance was repeated during this Git integration.
