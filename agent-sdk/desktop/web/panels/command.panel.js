/* Lane 4 命令面板：统一自然语言/多模态命令入口（子任务 2）。
 * 纯脚本 IIFE，注册 window.OwoPanels.command。防御性降级。
 */
window.OwoPanels = window.OwoPanels || {};
window.OwoPanels.command = (function () {
  "use strict";

  var id = "command";
  var H = null;
  var panelGeneration = 0;
  var auditGeneration = 0;
  var commandGeneration = 0;
  var commandBusy = false;
  var activeRunButton = null;
  function notify(message, kind) {
    if (H && H.notify) H.notify(message, kind || "error");
    else if (window.OwoToast) window.OwoToast(message);
    else window.alert(message);
  }

  function defaultHelpers() {
    var baseUrl = (window.OwoPanels && window.OwoPanels.baseUrl) || window.location.origin;
    function get(path) {
      return window.OwoApi.stream(path).then(function (r) {
        if (!r.ok) return r.json().catch(function () { return {}; }).then(function (body) {
          throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
        });
        return r.status === 204 ? null : r.json();
      });
    }
    function post(path, body) {
      return window.OwoApi.stream(path, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body || {}),
      }).then(function (r) {
        if (!r.ok) {
          return r.json().catch(function () { return {}; }).then(function (body) {
            throw new Error((body && (body.message || body.error)) || "HTTP " + r.status);
          });
        }
        return r.status === 204 ? null : r.json();
      });
    }
    function esc(s) {
      return String(s == null ? "" : s)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;");
    }
    function friendlyError(e) {
      return "操作失败：" + (e && e.message ? e.message : String(e));
    }
    function renderMarkdown(text) {
      return esc(text);
    }
    return { baseUrl: baseUrl, get: get, post: post, esc: esc, friendlyError: friendlyError, renderMarkdown: renderMarkdown };
  }

  function nav() {
    return (
      '<section data-panel="' + id + '">' +
      "<style>" +
      ".owo-command-log{height:180px;overflow:auto;background:var(--surface-2);color:var(--green);font-family:monospace;font-size:12px;padding:6px}" +
      ".owo-command-result{border:1px solid var(--border-strong);border-radius:6px;padding:6px;margin:4px 0;font-size:12px;background:var(--surface-2)}" +
      ".owo-command-tag{display:inline-block;padding:1px 8px;border-radius:8px;font-size:12px;background:var(--accent-soft);color:var(--accent);margin-right:6px}" +
      "</style>" +
      '<div class="stack">' +
      '<div class="sub">统一命令入口（文本 / 语音 / 区域 OCR）</div>' +
      '<div class="owo-command-row" style="display:flex;gap:8px;align-items:center">' +
      '<select id="owo-command-mode" style="padding:4px"><option value="text">文本</option>' +
      '<option value="voice">语音</option><option value="region">区域（OCR）</option></select>' +
      '<input id="owo-command-text" placeholder="例如：创建目标：整理桌面 / 搜索记忆：张子豪 / 运行工作流：报告" style="flex:1;padding:6px">' +
      '<button class="primary" id="owo-command-run" data-core-action>执行</button></div>' +
      '<input type="file" id="owo-command-wav" accept="audio/wav" style="display:none">' +
      '<div class="sub">意图</div><div id="owo-command-intent"></div>' +
      '<div class="sub">结果</div><div id="owo-command-results"></div>' +
      '<div class="sub">命令审计</div><div class="owo-command-log" id="owo-command-audit">（暂无）</div>' +
      "</div>"
    );
  }

  function mount(root, helpers) {
    dispose();
    H = helpers || defaultHelpers();
    root.innerHTML = nav();
    var runButton = root.querySelector("#owo-command-run");
    if (commandBusy) {
      activeRunButton = runButton;
      runButton.disabled = true;
      runButton.setAttribute("aria-busy", "true");
      runButton.textContent = "执行中…";
    }
    runButton.addEventListener("click", runCommand);
    root.querySelector("#owo-command-text").addEventListener("keydown", function (e) {
      if (e.key === "Enter") runCommand();
    });
    root.querySelector("#owo-command-mode").addEventListener("change", function (e) {
      var wav = document.getElementById("owo-command-wav");
      if (wav) wav.style.display = e.target.value === "voice" ? "inline-block" : "none";
    });
    refresh();
  }

  function refresh() {
    var request = ++auditGeneration;
    var owner = panelGeneration;
    return H.get("/command/audit")
      .then(function (data) {
        if (request !== auditGeneration || owner !== panelGeneration) return;
        var auditEl = document.getElementById("owo-command-audit");
        if (!auditEl) return;
        auditEl.textContent = (data && data.audit || [])
          .slice(0, 20)
          .map(function (a) {
            return String(a.event || "") + " — " + String(a.detail || "");
          })
          .join("\n");
      })
      .catch(function (error) {
        if (request !== auditGeneration || owner !== panelGeneration) return;
        var auditEl = document.getElementById("owo-command-audit");
        if (auditEl) auditEl.textContent = "审计记录加载失败：" + H.friendlyError(error);
      });
  }

  function readWavBase64(file) {
    return new Promise(function (resolve, reject) {
      var reader = new FileReader();
      reader.onload = function () {
        var base64 = String(reader.result).split(",")[1] || "";
        resolve(base64);
      };
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  }

  /// 区域 OCR：把屏幕上指定矩形里的文字识别出来当命令用（L2 视觉层需已授权）。
  function captureRegionToText() {
    var promptOptions = {
      title: "区域 OCR",
      label: "输入屏幕像素区域 x,y,width,height（例如 100,200,600,80）",
      value: "0,0,800,200",
      confirmText: "开始识别",
    };
    var requested;
    try {
      requested = H && H.prompt
        ? H.prompt(promptOptions)
        : Promise.resolve(window.prompt(promptOptions.label, promptOptions.value));
    } catch (error) {
      requested = Promise.reject(error);
    }
    return Promise.resolve(requested).then(function (raw) {
      if (!raw) return null;
      var parts = String(raw)
        .split(/[，,\s]+/)
        .filter(function (part) { return part.length; })
        .map(Number);
      if (parts.length !== 4 || parts.some(function (number) { return !isFinite(number) || number < 0; }) ||
          parts[2] <= 0 || parts[3] <= 0) {
        notify("请输入有效的 x,y,width,height；坐标不能为负，宽度和高度必须大于 0。", "error");
        return null;
      }
      return H.post("/perception/ocr/region", {
        x: parts[0], y: parts[1], width: parts[2], height: parts[3],
      }).then(function (data) {
        var text = data && (data.text || (data.lines || []).join("\n"));
        if (!text) {
          notify("该区域未识别到文字。", "warning");
          return null;
        }
        return text;
      });
    }).catch(function (e) {
      var msg = (e && e.message) || String(e);
      notify("区域 OCR 失败：" + msg + "。请确认已在设置中授权视觉识别。", "error");
      return null;
    });
  }

  function runCommand() {
    if (commandBusy) return Promise.resolve();
    var modeEl = document.getElementById("owo-command-mode");
    var textEl = document.getElementById("owo-command-text");
    var runButton = document.getElementById("owo-command-run");
    if (!modeEl || !textEl) return Promise.resolve();
    var mode = modeEl.value;
    var text = textEl.value;
    var owner = panelGeneration;
    var request = ++commandGeneration;
    commandBusy = true;
    activeRunButton = runButton;
    if (runButton) {
      runButton.disabled = true;
      runButton.setAttribute("aria-busy", "true");
      runButton.textContent = "执行中…";
    }

    function finish() {
      commandBusy = false;
      var buttonToRestore = activeRunButton || runButton;
      activeRunButton = null;
      if (buttonToRestore) {
        buttonToRestore.disabled = false;
        buttonToRestore.removeAttribute("aria-busy");
        buttonToRestore.textContent = "执行";
      }
    }
    function execute(body) {
      if (owner !== panelGeneration || request !== commandGeneration) return Promise.resolve();
      return postCommand(body, owner, request);
    }

    var operation;
    if (mode === "region") {
      operation = captureRegionToText().then(function (ocrText) {
        if (!ocrText || owner !== panelGeneration || request !== commandGeneration) return;
        modeEl.value = "text";
        textEl.value = ocrText;
        return execute({ mode: "text", text: ocrText });
      });
    } else if (mode === "voice") {
      var wavInput = document.getElementById("owo-command-wav");
      if (!wavInput || !wavInput.files || !wavInput.files.length) {
        notify("语音模式请先选择 WAV 文件。", "error");
        finish();
        return Promise.resolve();
      }
      operation = readWavBase64(wavInput.files[0])
        .then(function (wavB64) {
          return execute({ mode: "voice", wav_b64: wavB64 });
        })
        .catch(function (error) {
          if (owner === panelGeneration && request === commandGeneration) {
            notify("读取 WAV 文件失败：" + ((error && error.message) || error), "error");
          }
        });
    } else {
      if (!String(text || "").trim()) {
        notify("请先输入要执行的命令。", "error");
        finish();
        return Promise.resolve();
      }
      operation = execute({ mode: "text", text: text });
    }
    return Promise.resolve(operation).finally(finish);
  }

  function postCommand(body, owner, request) {
    var posted;
    try {
      posted = H.post("/command/run", body);
    } catch (error) {
      posted = Promise.reject(error);
    }
    return Promise.resolve(posted)
      .then(function (data) {
        if (owner !== panelGeneration || request !== commandGeneration) return;
        data = data || {};
        var confidence = Number(data.confidence);
        if (!isFinite(confidence)) confidence = 0;
        var intentEl = document.getElementById("owo-command-intent");
        if (intentEl) {
          intentEl.innerHTML =
            '<span class="owo-command-tag">' + H.esc(data.intent || "命令") + "</span>" +
            "置信度 " + confidence.toFixed(2) + " ｜ " + H.esc(data.text || "");
        }
        var resultsEl = document.getElementById("owo-command-results");
        if (resultsEl) resultsEl.innerHTML = renderResults(data.results || {});
        refresh();
      })
      .catch(function (e) {
        if (owner !== panelGeneration || request !== commandGeneration) return;
        var message = H.friendlyError(e);
        var intentEl = document.getElementById("owo-command-intent");
        if (intentEl) intentEl.innerHTML = '<span class="owo-command-tag">' + H.esc(message) + "</span>";
        notify(message, "error");
      });
  }

  function renderResults(results) {
    if (results && results.blocked) {
      return '<div class="owo-command-result"><b>已拦截</b>：' + H.esc(results.reason || "") + "</div>";
    }
    var lines = Object.keys(results).map(function (key) {
      var value = results[key];
      var rendered;
      if (Array.isArray(value)) {
        rendered = value
          .map(function (v) {
            var text = typeof v === "object" && v !== null ? JSON.stringify(v) : String(v);
            return H.esc(text);
          })
          .join("<br>");
      } else if (typeof value === "object" && value !== null) {
        rendered = H.renderMarkdown(JSON.stringify(value, null, 2));
      } else {
        rendered = H.esc(String(value));
      }
      return "<b>" + H.esc(key) + "：</b>" + rendered;
    });
    return lines.length ? lines.map(function (l) { return '<div class="owo-command-result">' + l + "</div>"; }).join("") : "";
  }

  function dispose() {
    panelGeneration += 1;
    auditGeneration += 1;
    commandGeneration += 1;
    // 命令请求无法随面板切换撤销；保持提交锁，防止重进页面重复执行。
  }

  return {
    id: id,
    title: "统一命令入口",
    nav: nav,
    mount: mount,
    refresh: refresh,
    dispose: dispose,
    _test: { renderResults: renderResults, runCommand: runCommand },
  };
})();
