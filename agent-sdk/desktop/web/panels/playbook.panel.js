// 任务剧本面板：列表 → 进度 → 逐步记录（pass/fail/skip）。
// 只调用 /playbooks 三条只读/记录接口；命令执行由 Agent 的 playbook_check 完成，
// 本面板不提供执行入口（权限/审批不绕）。
// IIFE 注册 window.OwoPanels.playbook；helpers 缺省时用 OwoApi。
(function () {
  "use strict";

  var win = typeof window !== "undefined" ? window : globalThis;
  win.OwoPanels = win.OwoPanels || {};

  win.OwoPanels.playbook = (function () {
    var ID = "playbook";
    var H = {};
    var rootEl = null;
    var generation = 0;
    var expandedPath = null;
    var lastList = null;

    function defaultEsc(text) {
      var div = document.createElement("div");
      div.textContent = text == null ? "" : String(text);
      return div.innerHTML;
    }
    function esc(text) {
      return H.esc ? H.esc(text) : defaultEsc(text);
    }
    function friendly(error) {
      if (H.friendlyError) return H.friendlyError(error);
      return String((error && error.message) || error || "未知错误");
    }
    function get(path) {
      if (H.get) return H.get(path);
      return window.OwoApi.stream(path).then(function (response) {
        if (!response.ok) {
          return response.text().then(function (body) {
            throw new Error(response.status + ": " + body);
          });
        }
        return response.json();
      });
    }
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (response) {
        if (!response.ok) {
          return response.text().then(function (text) {
            throw new Error(response.status + ": " + text);
          });
        }
        return response.json();
      });
    }

    function mark(state) {
      if (state === "passed") return "✔";
      if (state === "failed") return "✘";
      if (state === "skipped") return "⊘";
      return "○";
    }

    function nav() {
      return (
        '<section data-panel="playbook" class="owo-pb-panel">' +
        '<div class="owo-pb-tools"><h3>任务剧本</h3>' +
        '<button type="button" class="owo-pb-refresh" aria-label="刷新剧本列表">刷新</button></div>' +
        '<div class="hint">按剧本推进：查看进度、记录通过/失败/跳过。' +
        '需要自动执行 <code>check</code> 步骤时，让 Agent 调用 <code>playbook_check</code>（经审批）。</div>' +
        '<div class="owo-pb-new">' +
        '<input type="text" class="owo-pb-new-id" placeholder="新剧本 id（小写/数字/连字符）" aria-label="新剧本 id">' +
        '<button type="button" class="owo-pb-create" data-pb-create="1">新建剧本</button>' +
        "</div>" +
        '<div class="owo-pb-meta sub" role="status" aria-live="polite">加载中…</div>' +
        '<div class="owo-pb-list"></div>' +
        '<div class="owo-pb-error sub" role="alert" hidden></div>' +
        "</section>"
      );
    }

    function bar(progress) {
      var percent = Math.max(0, Math.min(100, Number(progress && progress.percent) || 0));
      return (
        '<div class="owo-pb-bar" role="progressbar" aria-valuenow="' + percent +
        '" aria-valuemin="0" aria-valuemax="100" aria-label="剧本进度"><span style="width:' +
        percent + '%"></span></div>'
      );
    }

    function card(item) {
      var progress = item.progress || {};
      var issues = item.issues || [];
      var ok = item.ok !== false;
      var next = item.next ? '<span class="owo-pb-chip">下一步：' + esc(item.next) + "</span>" : "";
      var summary = progress.summary || (ok ? "0/" + (item.steps || 0) : "不可用");
      var issueHtml = issues.length
        ? '<div class="owo-pb-issue">' +
          issues
            .map(function (issue) {
              return esc(issue.message || issue.code || "问题");
            })
            .join("；") +
          "</div>"
        : "";
      var expanded = expandedPath === item.path;
      return (
        '<article class="owo-pb-card" data-path="' + esc(item.path) + '">' +
        '<div class="owo-pb-card-head"><div><b>' + esc(item.name || item.id || "未命名") + "</b>" +
        '<div class="sub">' + esc(item.id || "") + " · " + esc(item.steps || 0) + " 步 · " +
        esc(summary) + "</div>" +
        (item.description ? '<div class="hint">' + esc(item.description) + "</div>" : "") +
        "</div>" +
        '<div class="owo-pb-card-actions">' +
        '<button type="button" class="owo-pb-archive" data-pb-archive="' + esc(item.path) +
        '" title="把当前进度保存为历史快照">保存快照</button>' +
        '<button type="button" class="owo-pb-reset" data-pb-reset="' + esc(item.path) +
        '" title="先保存快照，再清空进度">重置</button>' +
        '<button type="button" class="owo-pb-toggle" data-pb-toggle="' + esc(item.path) + '">' +
        (expanded ? "收起" : "打开") + "</button></div></div>" +
        bar(progress) +
        '<div class="owo-pb-chips">' + next +
        (item.warnings
          ? '<span class="owo-pb-chip owo-pb-warn">高风险命令 ' + item.warnings + " 条</span>"
          : "") +
        (progress.blocked ? '<span class="owo-pb-chip owo-pb-warn">阻塞 ' + progress.blocked + "</span>" : "") +
        (progress.failed ? '<span class="owo-pb-chip owo-pb-bad">失败 ' + progress.failed + "</span>" : "") +
        "</div>" +
        issueHtml +
        '<div class="owo-pb-detail" hidden></div>' +
        "</article>"
      );
    }

    function paintList(root, data) {
      var list = root.querySelector(".owo-pb-list");
      var meta = root.querySelector(".owo-pb-meta");
      var playbooks = (data && data.playbooks) || [];
      lastList = data;
      if (meta) {
        meta.textContent = playbooks.length
          ? "共 " + playbooks.length + " 个剧本 · " + (data.dir || "")
          : "工作区 playbooks/ 下暂无剧本";
      }
      if (!list) return;
      list.innerHTML = playbooks.length
        ? playbooks.map(card).join("")
        : '<div class="hint">还没有剧本：在 <code>playbooks/</code> 放一个 JSON，或用 ' +
          "<code>owo-agent playbook validate</code> 起步。</div>";
    }

    function stepRow(step) {
      var action = "";
      if (step.state === "pending" || step.state === "failed") {
        action =
          '<div class="owo-pb-actions">' +
          '<input type="text" class="owo-pb-note" data-pb-note="' + esc(step.id) +
          '" placeholder="证据/原因（可选）" aria-label="步骤 ' + esc(step.id) + ' 的备注">' +
          '<button type="button" class="owo-pb-pass" data-pb-record="' + esc(step.id) +
          '" data-pb-outcome="pass">通过</button>' +
          '<button type="button" class="owo-pb-fail" data-pb-record="' + esc(step.id) +
          '" data-pb-outcome="fail">失败</button>' +
          '<button type="button" class="owo-pb-skip" data-pb-record="' + esc(step.id) +
          '" data-pb-outcome="skip">跳过</button>' +
          "</div>";
      }
      var command = step.command
        ? '<div class="hint owo-pb-cmd">命令：<code>' + esc(step.command) + "</code>" +
          '<button type="button" class="owo-pb-copy" data-pb-copy="' + esc(step.command) +
          '" aria-label="复制步骤 ' + esc(step.id) + ' 的命令">复制</button>' +
          "<span>（由 Agent 的 playbook_check 执行）</span></div>"
        : "";
      var warnings = (step.warnings || [])
        .map(function (message) {
          return '<div class="hint owo-pb-warn">⚠ ' + esc(message) + "</div>";
        })
        .join("");
      var failureKind = step.failure_kind
        ? '<div class="hint">失败类型：<code>' + esc(step.failure_kind) + "</code></div>"
        : "";
      var retryBudget = step.max_attempts
        ? '<div class="hint">重试上限：' +
          esc(step.max_attempts) +
          " 次（已失败 " +
          esc(step.attempts || 0) +
          " 次）</div>"
        : "";
      return (
        '<div class="owo-pb-step owo-pb-' + esc(step.state) + '">' +
        '<div class="owo-pb-step-head"><span class="owo-pb-mark">' + mark(step.state) +
        "</span><b>" + esc(step.title || step.id) + "</b>" +
        '<span class="sub">' + esc(step.id) + " · " + esc(step.kind || "") + "</span></div>" +
        (step.note ? '<div class="hint">' + esc(step.note) + "</div>" : "") +
        (step.attempts ? '<div class="hint">已尝试 ' + step.attempts + " 次</div>" : "") +
        failureKind +
        retryBudget +
        command +
        warnings +
        action +
        "</div>"
      );
    }

    function paintDetail(cardEl, data) {
      var box = cardEl.querySelector(".owo-pb-detail");
      if (!box) return;
      var steps = (data && data.steps) || [];
      cardEl.__pbSteps = {};
      cardEl.__pbStepKinds = {};
      steps.forEach(function (step) {
        cardEl.__pbSteps[step.id] = step.state;
        if (step.failure_kind) cardEl.__pbStepKinds[step.id] = step.failure_kind;
      });
      box.hidden = false;
      box.innerHTML =
        '<div class="owo-pb-detail-head"><span class="sub">' +
        esc((data && data.progress && data.progress.summary) || "") +
        "</span></div>" +
        (steps.length ? steps.map(stepRow).join("") : '<div class="hint">没有步骤</div>') +
        '<div class="owo-pb-history" hidden></div>';
    }

    /// 纯渲染：某次归档 vs 当前状态的逐步差异。
    function compareRows(currentSteps, run, currentKinds) {
      var current = currentSteps || {};
      var kindsNow = currentKinds || {};
      var archiveSteps = (run && run.steps) || {};
      var archiveKinds = (run && run.step_kinds) || {};
      var changes = [];
      Object.keys(archiveSteps).forEach(function (stepId) {
        var before = archiveSteps[stepId];
        var after = current[stepId] || "pending";
        var kindBefore = archiveKinds[stepId];
        var kindAfter = kindsNow[stepId];
        if (before === after && kindBefore === kindAfter) return;
        changes.push({
          id: stepId,
          from: before,
          to: after,
          kindFrom: kindBefore,
          kindTo: kindAfter,
          regression: before === "passed" && after !== "passed",
        });
      });
      if (!changes.length) {
        return '<div class="hint">与当前状态一致（没有步骤变化）</div>';
      }
      var regressions = changes.filter(function (item) {
        return item.regression;
      }).length;
      return (
        '<div class="owo-pb-compare-list">' +
        changes
          .map(function (item) {
            var kindText = "";
            if (item.kindFrom || item.kindTo) {
              kindText =
                '<span class="sub">失败类型 ' +
                esc(item.kindFrom || "-") +
                " → " +
                esc(item.kindTo || "-") +
                "</span>";
            }
            return (
              '<div class="owo-pb-compare-row' +
              (item.regression ? " owo-pb-bad" : "") +
              '"><code>' +
              esc(item.id) +
              "</code><span>" +
              esc(item.from) +
              " → " +
              esc(item.to) +
              "</span>" +
              kindText +
              (item.regression ? '<span class="owo-pb-chip owo-pb-bad">退化</span>' : "") +
              "</div>"
            );
          })
          .join("") +
        (regressions ? '<div class="hint">' + regressions + " 个已通过步骤退化</div>" : "") +
        "</div>"
      );
    }

    /// 历史运行：列出归档；点击可对比该次归档与当前状态。
    function loadHistory(root, cardEl, path) {
      var box = cardEl.querySelector(".owo-pb-history");
      if (!box) return Promise.resolve();
      return get("/playbooks/history?file=" + encodeURIComponent(path))
        .then(function (data) {
          var runs = (data && data.runs) || [];
          var current = cardEl.__pbSteps || {};
          cardEl.__pbHistory = runs;
          box.hidden = false;
          if (!runs.length) {
            box.innerHTML =
              '<div class="hint">还没有历史运行（CLI：owo-agent playbook archive）</div>';
            return;
          }
          box.innerHTML =
            '<div class="owo-pb-detail-head"><span class="sub">历史运行 ' +
            runs.length +
            " 次（点击对比当前）</span></div>" +
            runs
              .map(function (run) {
                var regressions = [];
                var steps = run.steps || {};
                Object.keys(steps).forEach(function (stepId) {
                  if (steps[stepId] === "passed" && current[stepId] !== "passed") {
                    regressions.push(stepId);
                  }
                });
                var progress = run.progress || {};
                return (
                  '<button type="button" class="owo-pb-run" data-pb-compare="' +
                  esc(run.marker || "") +
                  '">' +
                  '<span class="owo-pb-run-marker">' + esc(run.marker || "?") + "</span>" +
                  '<span class="sub">' + esc(progress.summary || "") + "</span>" +
                  (run.complete ? '<span class="owo-pb-chip">已完成</span>' : "") +
                  (regressions.length
                    ? '<span class="owo-pb-chip owo-pb-bad">退化：' +
                      esc(regressions.join("、")) +
                      "</span>"
                    : "") +
                  "</button>"
                );
              })
              .join("") +
            '<div class="owo-pb-compare" hidden></div>';
        })
        .catch(function (error) {
          box.hidden = false;
          box.innerHTML = '<div class="hint">历史读取失败：' + esc(friendly(error)) + "</div>";
        });
    }

    /// 点击历史行：渲染「该归档 → 当前」的逐步差异。
    function renderCompare(cardEl, marker) {
      var box = cardEl.querySelector(".owo-pb-compare");
      if (!box) return;
      var runs = cardEl.__pbHistory || [];
      var run = runs.filter(function (item) {
        return item.marker === marker;
      })[0];
      if (!run) return;
      box.hidden = false;
      box.innerHTML =
        '<div class="owo-pb-detail-head"><span class="sub">' +
        esc(marker) +
        " → 当前</span></div>" +
        compareRows(cardEl.__pbSteps, run, cardEl.__pbStepKinds);
    }

    function showError(root, message) {
      notify(root, message, false);
    }

    function notify(root, message, ok) {
      var box = root.querySelector(".owo-pb-error");
      if (!box) return;
      box.hidden = !message;
      box.textContent = message || "";
      if (box.classList) box.classList.toggle("owo-pb-ok", Boolean(ok));
    }

    function toggle(root, path) {
      var cardEl = root.querySelector('.owo-pb-card[data-path="' + cssEscape(path) + '"]');
      if (!cardEl) return Promise.resolve();
      var box = cardEl.querySelector(".owo-pb-detail");
      var button = cardEl.querySelector(".owo-pb-toggle");
      if (expandedPath === path && box && !box.hidden) {
        expandedPath = null;
        box.hidden = true;
        if (button) button.textContent = "打开";
        return Promise.resolve();
      }
      expandedPath = path;
      if (button) button.textContent = "收起";
      return get("/playbooks/status?file=" + encodeURIComponent(path))
        .then(function (data) {
          paintDetail(cardEl, data);
          return loadHistory(root, cardEl, path);
        })
        .catch(function (error) {
          box.hidden = false;
          box.innerHTML = '<div class="hint">读取失败：' + esc(friendly(error)) + "</div>";
        });
    }

    function cssEscape(value) {
      return String(value).replace(/["\\]/g, "\\$&");
    }

    /// 保存当前进度为历史快照（POST /playbooks/archive），随后刷新列表与历史。
    function archiveSnapshot(root, path) {
      notify(root, "");
      return post("/playbooks/archive", { file: path })
        .then(function (data) {
          notify(root, "已保存快照：" + (data.marker || ""), true);
          return refresh(root);
        })
        .catch(function (error) {
          showError(root, "保存快照失败：" + friendly(error));
        });
    }

    /// 新建空白剧本（POST /playbooks/init），成功后清空输入并刷新列表。
    function createPlaybook(root) {
      var input = root.querySelector(".owo-pb-new-id");
      var id = input ? String(input.value || "").trim() : "";
      if (!id) {
        showError(root, "请输入剧本 id（小写字母/数字/连字符）");
        return Promise.resolve();
      }
      notify(root, "");
      return post("/playbooks/init", { id: id })
        .then(function (data) {
          if (input) input.value = "";
          notify(root, "已创建剧本：" + (data.path || id), true);
          return refresh(root);
        })
        .catch(function (error) {
          showError(root, "创建失败：" + friendly(error));
        });
    }

    /// 重置进度：服务端会先保存 `reset-<时间戳>` 快照再清空状态与派发日志。
    function resetProgress(root, path) {
      return post("/playbooks/reset", { file: path })
        .then(function (data) {
          notify(root, "已重置，快照：" + (data.snapshot || ""), true);
          return refresh(root);
        })
        .catch(function (error) {
          showError(root, "重置失败：" + friendly(error));
        });
    }

    function record(root, path, step, outcome) {
      var cardEl = root.querySelector('.owo-pb-card[data-path="' + cssEscape(path) + '"]');
      var input = cardEl
        ? cardEl.querySelector('.owo-pb-note[data-pb-note="' + cssEscape(step) + '"]')
        : null;
      var note = input ? String(input.value || "").trim() : "";
      showError(root, "");
      return post("/playbooks/record", {
        file: path,
        step: step,
        outcome: outcome,
        note: note,
      })
        .then(function () {
          return refresh(root);
        })
        .catch(function (error) {
          showError(root, "记录失败：" + friendly(error));
        });
    }

    function bind(root) {
      root.addEventListener("click", function (event) {
        var target = event.target;
        if (!target || !target.closest) return;
        var toggleButton = target.closest("[data-pb-toggle]");
        if (toggleButton) {
          toggle(root, toggleButton.getAttribute("data-pb-toggle"));
          return;
        }
        var recordButton = target.closest("[data-pb-record]");
        if (recordButton) {
          var cardEl = recordButton.closest(".owo-pb-card");
          if (!cardEl) return;
          record(
            root,
            cardEl.getAttribute("data-path"),
            recordButton.getAttribute("data-pb-record"),
            recordButton.getAttribute("data-pb-outcome")
          );
          return;
        }
        var copyButton = target.closest("[data-pb-copy]");
        if (copyButton) {
          var command = copyButton.getAttribute("data-pb-copy") || "";
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(command).then(
              function () {
                notify(root, "已复制命令：" + command, true);
              },
              function () {
                notify(root, "复制失败，请手动选择命令", false);
              }
            );
          } else {
            notify(root, "当前环境不支持剪贴板，请手动选择命令", false);
          }
          return;
        }
        var archiveButton = target.closest("[data-pb-archive]");
        if (archiveButton) {
          archiveSnapshot(root, archiveButton.getAttribute("data-pb-archive"));
          return;
        }
        var resetButton = target.closest("[data-pb-reset]");
        if (resetButton) {
          var resetCard = resetButton.closest(".owo-pb-card");
          if (!resetCard) return;
          if (resetCard.getAttribute("data-reset-armed") === "1") {
            resetProgress(root, resetCard.getAttribute("data-path"));
          } else {
            resetCard.setAttribute("data-reset-armed", "1");
            resetButton.textContent = "确认重置？";
            notify(root, "再次点击「确认重置？」：会先保存快照再清空进度", false);
          }
          return;
        }
        var compareButton = target.closest("[data-pb-compare]");
        if (compareButton) {
          var compareCard = compareButton.closest(".owo-pb-card");
          if (compareCard) renderCompare(compareCard, compareButton.getAttribute("data-pb-compare"));
          return;
        }
        if (target.closest("[data-pb-create]")) {
          createPlaybook(root);
        }
      });
    }

    function refresh(root) {
      root = root || rootEl;
      if (!root) return Promise.resolve();
      var current = ++generation;
      var meta = root.querySelector(".owo-pb-meta");
      if (meta) meta.textContent = "加载中…";
      return get("/playbooks")
        .then(function (data) {
          if (current !== generation || rootEl !== root) return;
          paintList(root, data);
          if (expandedPath) {
            var cardEl = root.querySelector(
              '.owo-pb-card[data-path="' + cssEscape(expandedPath) + '"]'
            );
            if (cardEl) {
              return get("/playbooks/status?file=" + encodeURIComponent(expandedPath)).then(
                function (statusData) {
                  paintDetail(cardEl, statusData);
                  return loadHistory(root, cardEl, expandedPath);
                }
              );
            }
          }
          return null;
        })
        .catch(function (error) {
          if (current !== generation || rootEl !== root) return;
          if (meta) meta.textContent = "加载失败，可重试";
          showError(root, "剧本列表加载失败：" + friendly(error));
        });
    }

    function mount(root, helpers) {
      dispose();
      H = {
        get: helpers && helpers.get,
        friendlyError: helpers && helpers.friendlyError,
        esc: helpers && helpers.esc,
      };
      if (!root) return;
      rootEl = root;
      root.innerHTML = nav();
      bind(root);
      var button = root.querySelector(".owo-pb-refresh");
      if (button)
        button.addEventListener("click", function () {
          refresh(root);
        });
      refresh(root);
    }

    function dispose() {
      generation += 1;
      rootEl = null;
      expandedPath = null;
      lastList = null;
    }

    var TEST_API = {
      card: card,
      stepRow: stepRow,
      mark: mark,
      bar: bar,
      compareRows: compareRows,
    };

    return {
      id: ID,
      title: "任务剧本",
      nav: nav,
      mount: mount,
      refresh: refresh,
      dispose: dispose,
      _test: TEST_API,
      get _lastList() {
        return lastList;
      },
    };
  })();
})();

if (typeof module !== "undefined" && module.exports) {
  var __pbWin = typeof window !== "undefined" ? window : globalThis;
  module.exports = __pbWin.OwoPanels.playbook;
}
