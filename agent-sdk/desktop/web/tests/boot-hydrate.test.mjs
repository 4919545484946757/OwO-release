import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const app = readFileSync(join(here, "../app.js"), "utf8");
const appDomain = readFileSync(join(here, "../app-domain.js"), "utf8");
const index = readFileSync(join(here, "../index.html"), "utf8");
const client = readFileSync(join(here, "../core/api-client.js"), "utf8");
const serverEvents = readFileSync(join(here, "../../../crates/owo-agent-server/src/event_stream.rs"), "utf8");

test("首屏先健康检查，再只水合会话、设置、技能与白名单", () => {
  const tasks = app.match(/const BOOT_HYDRATE_TASKS = \[([^\]]+)\];/);
  assert.ok(tasks);
  assert.deepEqual([...tasks[1].matchAll(/\b(refresh[A-Z]\w*)\b/g)].map(match => match[1]), [
    "refreshSessions", "refreshSettings", "refreshSkills", "refreshWhitelist",
  ]);
  const hydrate = app.match(/async function hydrateShell\(\) \{[\s\S]*?\n\}/);
  assert.ok(hydrate);
  assert.match(hydrate[0], /await serviceWatch\.start\(\)/);
  assert.match(hydrate[0], /runWithConcurrency\(BOOT_HYDRATE_TASKS, 4\)/);
  assert.match(app, /await hydrateShell\(\);[\s\S]*?await restoreLastSession\(\);/);
});

test("steady refresh is bounded; wide domain refresh is reserved for degraded SSE fallback", () => {
  const plans = app.match(/const WORKBENCH_REFRESH_PLANS = \[([\s\S]*?)\n\];/);
  assert.ok(plans);
  assert.equal([...plans[1].matchAll(/\{ refresh:/g)].length, 3);
  assert.match(plans[1], /refreshHealth, intervalMs: 30000/);
  assert.match(plans[1], /refreshPerception, intervalMs: 30000/);
  assert.match(plans[1], /refreshPetState, intervalMs: 60000/);
  assert.match(app, /pollIntervalMs: 600000/);
  assert.match(app, /next\.setPollFallback\(refreshAllInvalidatedDomains\)/);
  assert.match(app, /concurrency: 2,\s*isHidden: uiHidden/);
});

test("active entrypoint wires every server domain to the authenticated invalidation stream", () => {
  assert.match(index, /<script src="core\/events\.js"><\/script>/);
  const handlerBlock = app.match(/const INVALIDATE_HANDLERS = Object\.freeze\(\{([\s\S]*?)\n\}\);/);
  assert.ok(handlerBlock);
  const frontendDomains = [...handlerBlock[1].matchAll(/^\s{2}(\w+):/gm)].map(match => match[1]).sort();
  const all = serverEvents.match(/pub const ALL: &\'static \[InvalidateDomain\] = &\[([\s\S]*?)\];/);
  assert.ok(all);
  const variants = [...all[1].matchAll(/Self::(\w+)/g)].map(match => match[1]);
  const names = serverEvents.match(/pub fn as_str\(self\) -> &\'static str \{([\s\S]*?)\n    \}/);
  assert.ok(names);
  const wireNames = new Map([...names[1].matchAll(/Self::(\w+) => "([^"]+)"/g)].map(match => [match[1], match[2]]));
  assert.deepEqual(frontendDomains, variants.map(name => wireNames.get(name)).sort());
  assert.match(app, /openStream: \(path, options\) => apiClient\.openEventStream\(path, options\)/);
  assert.match(app, /apiClient\.coreInstanceId \|\| ""/);
});

test("polling and event handlers suspend in native desktop background and flush on wake", () => {
  assert.match(app, /function uiHidden\(\)[\s\S]*shellBackgroundHidden[\s\S]*document\.visibilityState/);
  const setter = app.match(/window\.owoSetBackground = function \(hidden\) \{([\s\S]*?)\n\};/);
  assert.ok(setter);
  assert.match(setter[1], /setShellBackground/);
  assert.match(setter[1], /invalidator\?\.onVisibility\(\)/);
  assert.match(setter[1], /workbenchRefresh\?\.wake\(\)/);
  assert.match(app, /if \(!uiHidden\(\)\) return refresh\(\);/);
});

test("connection loss stops SSE and a new Core instance rebuilds the stream", () => {
  assert.match(app, /function markConnectionUnavailable\(error\) \{[\s\S]*?stopInvalidation\(\);/);
  assert.match(app, /function markConnectionReady\(authenticated\)[\s\S]*?if \(shellHydrated && !authFailed\) startInvalidation\(\)/);
  assert.match(app, /window\.addEventListener\("owo:connection"[\s\S]*?startInvalidation\(\)[\s\S]*?stopInvalidation\(\)/);
  assert.match(app, /if \(invalidator && invalidationKey === key\) return/);
  assert.match(client, /handleNetworkFailure\(allowRetry\)[\s\S]{0,700}resetCoreConnection\(\)/);
  assert.match(client, /REHANDSHAKE_COOLDOWN_MS/);
});


test("hidden inspector panels are mounted on demand, not during chat startup", () => {
  const init = appDomain.match(/function initPanels\(\) \{[\s\S]*?\n\}/);
  assert.ok(init);
  assert.match(init[0], /button\.addEventListener\("click", \(\) => openPanelById\(id\)\)/);
  assert.doesNotMatch(init[0], /mountPanel\(first/);
  assert.match(init[0], /panel-empty-state/);
});
