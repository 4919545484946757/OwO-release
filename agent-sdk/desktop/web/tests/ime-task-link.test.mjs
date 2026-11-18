import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
const { create, fromSearch } = createRequire(import.meta.url)("../core/ime-task-link.js");
const id = "11111111-1111-4111-8111-111111111111";
test("task links wait for hydration and only navigate to the unified panel", async () => {
  let ready = false;
  const calls = [];
  const link = create({ isReady: () => ready, openPanel: value => {calls.push(["panel", value]);return true;},
    openTask: async value => calls.push(["task", value]) });
  assert.equal(await link.receive({taskId:id}), false);
  assert.deepEqual(calls, []);
  ready = true;
  assert.equal(await link.flush(), true);
  assert.deepEqual(calls, [["panel","agent-tasks"],["task",id]]);
});
test("invalid task references and ambiguous query parameters never navigate", async () => {
  const calls = [];
  const link = create({isReady:()=>true,openPanel:()=>calls.push("panel"),openTask:()=>calls.push("task")});
  for (const taskId of ["../../session/x", "https://example.invalid", "<script>", "------------------------------------", 42]) {
    assert.equal(await link.receive({taskId}),false);
  }
  assert.deepEqual(calls,[]);
  assert.equal(fromSearch("?panel=agent-tasks&task="+id+"&task="+id),null);
  assert.equal(fromSearch("?panel=agent-tasks&task=../x"),null);
  assert.deepEqual(fromSearch("?panel=agent-tasks&task="+id),{taskId:id});
  assert.deepEqual(fromSearch("?panel=agent-tasks"),{taskId:null});
});
test("opening the list does not open or execute a task; errors are surfaced", async () => {
  const calls=[];const errors=[];
  const link=create({isReady:()=>true,openPanel:x=>calls.push(x),openTask:()=>{throw new Error("unavailable");},onError:e=>errors.push(e.message)});
  assert.equal(await link.receive({taskId:null}),true);
  assert.deepEqual(calls,["agent-tasks"]);
  assert.equal(await link.receive({taskId:id}),false);
  assert.deepEqual(errors,["unavailable"]);
});
