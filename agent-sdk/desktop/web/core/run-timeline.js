// One authenticated, resumable observation channel. No transcript or scheduler lives here.
(function (root) {
  "use strict";
  function create(options) {
    var o = options || {}, run = String(o.runId || "");
    if (!run || !o.api) throw new Error("Timeline requires a run and API client");
    var api = o.api, cursor = 0, generation = 0, running = false;
    var controller = null, timer = null, failures = 0;
    var setTimer = o.setTimeout || setTimeout, clearTimer = o.clearTimeout || clearTimeout;
    var path = "/teams/" + encodeURIComponent(run) + "/timeline";
    function status(mode, error) {
      if (o.onState) o.onState({ mode: mode, error: error || null, cursor: cursor });
    }
    function live(g) { return running && generation === g; }
    function fail(message) {
      var error = new Error(message);
      error.timelineProtocol = true;
      throw error;
    }
    function accept(record, frameId) {
      if (!record || record.schema_version !== 1 || record.run_id !== run ||
          !Number.isSafeInteger(record.seq) || record.seq <= 0 ||
          record.event_id !== "run:" + run + ":" + record.seq ||
          (frameId != null && frameId !== record.event_id) ||
          typeof record.event_type !== "string") fail("运行事件身份或协议无效");
      if (record.seq <= cursor) return; // at-least-once delivery; gaps across runs are legal
      if (o.onEvent) o.onEvent(record); // acknowledge only after the consumer accepts it
      cursor = record.seq;
    }
    function metadata(value, page) {
      if (!value || value.run_id !== run || (!page && value.schema_version !== 1)) fail("运行历史元信息无效");
      if (o.onMeta) o.onMeta({
        run_id: run, schema_version: 1, high_watermark: value.high_watermark,
        history_complete: value.history_complete, task_history_complete: value.task_history_complete,
      });
    }
    function stop() {
      running = false;
      generation += 1;
      if (timer != null) { clearTimer(timer); timer = null; }
      if (controller) { controller.abort(); controller = null; }
    }
    function schedule(g, fn, delay) {
      if (!live(g)) return;
      timer = setTimer(function () { timer = null; if (live(g)) fn(g); }, delay);
    }
    function failed(g, error) {
      if (!live(g)) return;
      var code = Number(error && error.status);
      if (code === 404 || code === 501) {
        stop();
        status("unsupported", error);
        if (o.onUnsupported) o.onUnsupported(error);
      } else if ((error && error.timelineProtocol) || [400, 401, 403].indexOf(code) >= 0) {
        stop();
        status("error", error); // never reset an invalid cursor or hide authentication errors
      } else {
        failures += 1;
        status("poll", error);
        schedule(g, catchUp, Math.min(10000, 1000 * Math.pow(2, Math.min(3, failures - 1))));
      }
    }
    async function catchUp(g) {
      if (!live(g)) return;
      var ctrl = new AbortController();
      controller = ctrl;
      try {
        // Yield between bounded batches even when a durable outbox has more work than this page.
        for (var n = 0; n < 8; n += 1) {
          var page = await api.get(path + "?format=json&limit=64&after=" + cursor, { signal: ctrl.signal });
          if (!live(g)) return;
          metadata(page, true);
          if (!Array.isArray(page.events) || page.events.length > 64 || typeof page.has_more !== "boolean") {
            fail("运行历史分页无效");
          }
          var previous = cursor;
          page.events.forEach(function (record) {
            if (record.seq <= previous) fail("运行历史分页顺序无效");
            accept(record);
            previous = record.seq;
          });
          if (!page.has_more) {
            var detail = await api.get("/teams/" + encodeURIComponent(run), { signal: ctrl.signal });
            if (!live(g)) return;
            if (o.onSnapshot) o.onSnapshot(detail);
            var teamStatus = String(detail && detail.team && detail.team.status || "").toLowerCase();
            if (["succeeded", "failed", "cancelled", "aborted"].indexOf(teamStatus) >= 0) {
              // A terminal snapshot is followed by one final persisted read: terminal and its
              // event may have committed after the empty page above.
              var tail = await api.get(path + "?format=json&limit=64&after=" + cursor, { signal: ctrl.signal });
              if (!live(g)) return;
              metadata(tail, true);
              if (!Array.isArray(tail.events) || tail.events.length > 64 || typeof tail.has_more !== "boolean") fail("运行历史尾页无效");
              previous = cursor;
              tail.events.forEach(function (record) {
                if (record.seq <= previous) fail("运行历史尾页顺序无效");
                accept(record); previous = record.seq;
              });
              if (tail.has_more) continue;
              stop(); status("done"); return;
            }
            failures = 0;
            schedule(g, connect, 1000);
            return;
          }
        }
        schedule(g, catchUp, 0);
      } catch (error) { ctrl.abort(); failed(g, error); }
      finally { if (controller === ctrl) controller = null; }
    }
    async function connect(g) {
      if (!live(g)) return;
      if (typeof api.openEventStream !== "function") { status("poll"); return catchUp(g); }
      var ctrl = new AbortController();
      controller = ctrl;
      try {
        status("connecting");
        await api.openEventStream(path + "?format=sse&limit=64&after=" + cursor, {
          signal: ctrl.signal,
          lastEventId: cursor ? "run:" + run + ":" + cursor : undefined,
          onOpen: function () { if (live(g)) { failures = 0; status("sse"); } },
          onEvent: function (frame) {
            if (!live(g)) return;
            if (frame.event === "timeline_error") throw new Error("运行历史暂不可读，将从已接收游标重试");
            if (frame.event === "timeline" || frame.event === "run_event") {
              var value;
              try { value = JSON.parse(frame.data); } catch (_) { fail("运行事件 JSON 无效"); }
              if (frame.event === "timeline") metadata(value);
              else accept(value, frame.id);
            }
          },
        });
        if (live(g)) await catchUp(g); // EOF alone is not proof of a terminal run
      } catch (error) { ctrl.abort(); failed(g, error); }
      finally { if (controller === ctrl) controller = null; }
    }
    return {
      start: function () { if (running) return; running = true; connect(++generation); },
      stop: stop,
      cursor: function () { return cursor; },
    };
  }
  var exported = { create: create };
  root.OwoRunTimeline = exported;
  if (typeof module !== "undefined" && module.exports) module.exports = exported;
})(typeof window !== "undefined" ? window : globalThis);
