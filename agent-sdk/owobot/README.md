# OwOBot development

OwOBot reuses the pinned OpenMausBot front end and collaboration server. See [source provenance](UPSTREAM.md) for the upstream revision, Apache license and notices.

## Build the collaboration components

Use Windows x64 with Node.js 24.22.3 and pnpm 10.33.0. Run the commands from the sibling `upstream` directory.

```powershell
pnpm install --frozen-lockfile --ignore-scripts --filter openmausbot
pnpm run typecheck
pnpm exec vitest run server/request-auth.test.ts
pnpm exec vite build --config vite.owobot.config.ts
pnpm run build:server
```

The OwOBot workbench is in `upstream/dist-owobot`. The local collaboration bundle is in `upstream/dist-server`.

## Verify the local service and ACP bridge

Build `owo-agent` from this worktree under the repository's Rust resource policy. From the repository root, run the following with Node.js 24 on the path.

```powershell
python agent-sdk/owobot/tests/windows_smoke.py --node (Get-Command node.exe).Source --agent agent-sdk/target/debug/owo-agent.exe
python agent-sdk/owobot/tests/windows_smoke.py --node (Get-Command node.exe).Source --agent agent-sdk/target/debug/owo-agent.exe --bundled
```

Each run uses separate temporary directories. The smoke test pairs with the managed collaboration process, creates a group, and checks that a message opens an OwO session for the assigned project. It verifies that unpaired loopback clients cannot access project messages and that a model-disabled turn reaches a terminal failure. The test prints the location of its logs. Live model delegation and Windows installer acceptance are separate checks.

## Verify the desktop workbench

With the workbench and server bundles built, run from the repository root.

```powershell
python agent-sdk/owobot/tests/desktop_gateway.py --node (Get-Command node.exe).Source --agent agent-sdk/target/debug/owo-agent.exe --electron agent-sdk/desktop/electron/node_modules/electron/dist/electron.exe
```

The test exercises the OwO Daemon's managed child process, project-scoped browser cookie, server events, and shutdown. The optional Electron run opens the OwO desktop page, creates a collaborative task, and exercises light, dark and system appearance through the desktop preference. It records two screenshots in the isolated test directory. The model is disabled in this smoke test.

A development shell can use `OWOBOT_HOME` pointing at a staged runtime with `node.exe`, `dist/index.html`, and `dist-server/index.js`. The smoke test can also use a staged runtime via `--runtime`.

The current ACP path supports scoped project reading and in-room assignments by the project lead. Approval and artifact delivery are subsequent integration steps.

## Verify the selected desktop model

The loopback fixture exercises the full group message, ACP, OwO Daemon and streaming model path. It asserts that the selected model and its bound credential reach the endpoint, that each of the three roles receives its own project readers, and that a forged worker capability cannot launch a turn. The fixture then drives a lead, worker and reviewer relay through model tool calls. It checks a project file read, both member replies, the lead summary, two audited assignments and a repeated send ID. It uses a synthetic key and a local HTTP endpoint.

    python agent-sdk/owobot/tests/desktop_gateway.py --runtime agent-sdk/desktop/electron/binaries/owobot --agent agent-sdk/target/debug/owo-agent.exe --verify-model

## Stage the Windows runtime

The Electron distribution stages the OwOBot runtime with the desktop resources. For a development run, stage the same runtime from the repository root.

```powershell
& .\agent-sdk\scripts\stage-owobot.ps1
$env:OWOBOT_HOME = (Resolve-Path .\agent-sdk\desktop\electron\binaries\owobot).Path
```

The staging script checks the official Windows x64 Node archive against its pinned SHA-256, builds with the pinned pnpm and lockfile, and publishes a build manifest and the applicable upstream notices. To reuse a local copy of the checked archive and pnpm CLI, set OWOBOT_NODE_ARCHIVE and OWOBOT_PNPM_CLI.
