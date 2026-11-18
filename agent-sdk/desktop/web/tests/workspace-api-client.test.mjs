// 工作区客户端契约：路径、响应形状与错误透传。
// 运行：node --test tests/workspace-api-client.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../core/workspace-api-client.js", import.meta.url), "utf8");

function harness() {
  const window = {};
  vm.runInNewContext(source, { window, globalThis: window }, { filename: "workspace-api-client.js" });
  const calls = [];
  const lease = {
    workspace_id: "ws-1",
    task_id: "t-1",
    kind: "git_worktree",
    state: "prepared",
    path: "D:/data/workspaces/owo-ws-1",
    base_commit: "abc",
  };
  const transport = {
    get: async (path) => {
      calls.push(["GET", path]);
      if (path === "/workspaces") return { ok: true, workspaces: [lease, null] };
      if (path.endsWith("/diff")) {
        return { ok: true, diff: { workspace_id: "ws-1", state: "prepared", patch: "diff --git", files: [] }, empty: false };
      }
      return { ok: true, workspace: lease };
    },
    post: async (path, body) => {
      calls.push(["POST", path, body]);
      if (path === "/workspaces") return { ok: true, workspace: lease };
      return { ok: true, workspace: lease, applied: true };
    },
    del: async (path) => {
      calls.push(["DELETE", path]);
      return { ok: true, workspace: Object.assign({}, lease, { state: "released" }) };
    },
  };
  return { client: window.OwoWorkspaceApi.createWorkspaceApiClient(transport), calls, lease };
}

const local = (value) => JSON.parse(JSON.stringify(value));

test("list drops invalid leases and reads the documented envelope", async () => {
  const { client, calls } = harness();
  const data = await client.list();
  assert.deepEqual(calls, [["GET", "/workspaces"]]);
  assert.equal(data.workspaces.length, 1);
  assert.equal(data.workspaces[0].workspace_id, "ws-1");
});

test("diff understands the diff envelope and empty flag", async () => {
  const { client, calls } = harness();
  const result = await client.diff("ws-1");
  assert.deepEqual(calls, [["GET", "/workspaces/ws-1/diff"]]);
  assert.equal(result.empty, false);
  assert.match(result.diff.patch, /diff --git/);
});

test("lifecycle actions hit apply/reject/rollback/release paths", async () => {
  const { client, calls } = harness();
  await client.apply("ws-1");
  await client.reject("ws-1");
  await client.rollback("ws-1");
  await client.release("ws-1");
  assert.deepEqual(local(calls), [
    ["POST", "/workspaces/ws-1/apply", {}],
    ["POST", "/workspaces/ws-1/reject", {}],
    ["POST", "/workspaces/ws-1/rollback", {}],
    ["DELETE", "/workspaces/ws-1"],
  ]);
});

test("ids are encoded and missing ids fail before transport", async () => {
  const { client, calls } = harness();
  await client.get("ws/1");
  assert.deepEqual(calls, [["GET", "/workspaces/ws%2F1"]]);
  assert.throws(() => client.diff(""), (error) => error.code === "INVALID_REQUEST");
});

test("incomplete responses are rejected instead of rendered", async () => {
  const window = {};
  vm.runInNewContext(source, { window, globalThis: window }, { filename: "workspace-api-client.js" });
  const client = window.OwoWorkspaceApi.createWorkspaceApiClient({
    get: async () => ({ ok: true, workspaces: "nope" }),
    post: async () => ({ ok: true }),
    del: async () => ({ ok: true }),
  });
  await assert.rejects(() => client.list(), (error) => error.code === "INCOMPLETE_RESPONSE");
});
