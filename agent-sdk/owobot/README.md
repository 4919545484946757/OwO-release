# OwOBot development

OwOBot reuses the pinned OpenMausBot front end and collaboration server. See [source provenance](UPSTREAM.md) for the upstream revision, Apache license and notices.

## Build the collaboration components

Use Windows x64 with Node.js 24.22.3 and pnpm 10.33.0. Run the commands from the sibling `upstream` directory.

```powershell
pnpm install --frozen-lockfile --ignore-scripts --filter openmausbot
pnpm run typecheck
pnpm exec vitest run server/request-auth.test.ts
pnpm exec vite build --base /owobot/
pnpm run build:server
```

The React output is in `upstream/dist`. The local collaboration bundle is in `upstream/dist-server`.

## Verify the local service and ACP bridge

Build `owo-agent` from this worktree under the repository's Rust resource policy. From the repository root, run the following with Node.js 24 on the path.

```powershell
python agent-sdk/owobot/tests/windows_smoke.py --node (Get-Command node.exe).Source --agent agent-sdk/target/debug/owo-agent.exe
python agent-sdk/owobot/tests/windows_smoke.py --node (Get-Command node.exe).Source --agent agent-sdk/target/debug/owo-agent.exe --bundled
```

Each run uses separate temporary directories. The smoke test pairs with the managed collaboration process, creates a group, and checks that a message opens an OwO session for the assigned project. It verifies that unpaired loopback clients cannot access project messages and that a model-disabled turn reaches a terminal failure. The test prints the location of its logs. Live model delegation and Windows installer acceptance are separate checks.

The current ACP path supports text in read-only OwO turns. The desktop launch and the project-scoped delegation, approval and delivery flows are still under development.
