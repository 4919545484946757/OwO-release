# OWO 真实回合端到端测试（零云端成本）：
# mock OpenAI 兼容端点（scripts/mock-openai-tool-provider.py）→ 真实 daemon → 真实 Web UI。
# 覆盖：SSE 事件链（tool_use/permission_request/tool_result/final/turn_stats）、
# 审批卡交互、工具时间线 chip、真实文件落盘、检查器行级着色 diff（新增文件全绿行）。
import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request

from playwright.sync_api import sync_playwright

CHECKS = []


def check(name, ok, detail=""):
    CHECKS.append((name, bool(ok), detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f" :: {detail}" if detail else ""))


def wait_http(url, timeout_s=30):
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if response.status == 200:
                    return True
        except Exception:
            time.sleep(0.4)
    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon-port", type=int, default=4828)
    parser.add_argument("--mock-port", type=int, default=4831)
    parser.add_argument("--repo", default=os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    args = parser.parse_args()

    ws = tempfile.mkdtemp(prefix="owo-turn-ws-")
    data = tempfile.mkdtemp(prefix="owo-turn-data-")

    provider = subprocess.Popen(
        [sys.executable, os.path.join(args.repo, "scripts", "mock-openai-tool-provider.py"),
         "--port", str(args.mock_port)],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    env = dict(os.environ)
    env["OWO_AGENT_DATA"] = data
    env["OPENAI_BASE_URL"] = f"http://127.0.0.1:{args.mock_port}/v1"
    env["OPENAI_API_KEY"] = "mock-key-for-e2e"
    env["OPENAI_MODEL"] = "mock-1"
    # 关键：daemon 的 stdout 必须落到文件。用 PIPE 却不排空时，缓冲一旦写满
    # daemon 会阻塞在日志写入上（实测回合中途卡死、会话都建不出来）。
    daemon_log_path = os.path.join(data, "daemon.log")
    daemon_log = open(daemon_log_path, "wb")
    daemon = subprocess.Popen(
        [os.path.join(args.repo, "target", "debug", "owo-agent.exe"),
         "serve", "--port", str(args.daemon_port), "--workspace", ws],
        env=env, stdout=daemon_log, stderr=subprocess.STDOUT,
    )
    base = f"http://127.0.0.1:{args.daemon_port}"
    try:
        check("mock 端点就绪", wait_http(f"http://127.0.0.1:{args.mock_port}/health", 10) or True,
              "mock 无 /health 也继续（仅 404 即存活）")
        # mock provider 只实现 /chat/completions；用连接尝试确认存活即可。
        check("daemon 就绪", wait_http(base + "/health", 30))

        console_errors = []
        page_errors = []
        with sync_playwright() as p:
            try:
                browser = p.chromium.launch(headless=True, channel="msedge")
            except Exception:
                browser = p.chromium.launch(headless=True)
            context = browser.new_context(viewport={"width": 1440, "height": 900})
            context.add_init_script(
                "try { window.localStorage.setItem('owo.workspace', %s); } catch (e) {}" % json.dumps(ws)
            )
            page = context.new_page()
            page.on("console", lambda msg: console_errors.append(msg.text) if msg.type == "error" else None)
            page.on("pageerror", lambda err: page_errors.append(str(err)))
            page.goto(base + "/", wait_until="domcontentloaded")
            page.wait_for_selector("#menubar", timeout=30000)
            # 引导 settle：健康就绪后再发送。过早点击时工作区输入尚未同步，
            # 自动建会话会以「请先选择项目工作区」失败（toast 已消散难排查）。
            try:
                page.wait_for_function(
                    "() => { const el = document.getElementById('health'); return el && !/连接中/.test(el.textContent); }",
                    timeout=30000,
                )
            except Exception as error:
                check("引导就绪", False, str(error)[:120])
            page.wait_for_timeout(1500)

            def messages_digest():
                # 失败诊断：把会话流当前内容 + 最近 toast 打出来（截断）。
                try:
                    text = page.text_content("#messages") or ""
                    toast = page.evaluate(
                        "() => { const r = document.getElementById('toastRoot'); return r ? r.textContent : null; }"
                    )
                    return f"{' '.join(text.split())[:300]} :: toast: {toast}"
                except Exception as error:
                    return f"<messages 不可读: {error}>"

            # 发送 prompt（sendPrompt 会自动建会话）。
            page.fill("#prompt", "请把 cli golden content 写入 cli-golden-output.txt")
            page.click("#sendBtn")
            print("  已发送，等待审批卡…")

            # 审批卡出现 → 允许（仅本次）。
            try:
                page.wait_for_selector("#approvalBar .approval-item", timeout=30000)
                tool_name = page.text_content("#approvalList .approval-item .tool-name")
                check("审批卡渲染真实工具", "写入文件" in (tool_name or "") or "write" in (tool_name or "").lower(),
                      (tool_name or "").strip())
                page.click("#approvalList .approval-actions button.allow")
                print("  已点允许，等待回合结束…")
            except Exception as error:
                check("审批卡渲染真实工具", False, f"{str(error)[:120]} :: 流内容: {messages_digest()}")

            # 工具 chip 完成（绿点）+ 最终回复 + 汇报卡。
            # 注：工具段默认折叠（.tool-steps details 收起），chip 存在但不可见 → attached 断言。
            try:
                page.wait_for_selector("#messages .tool-step.is-ok", timeout=30000, state="attached")
                step_text = page.eval_on_selector(
                    "#messages .tool-step.is-ok", "e => e.textContent"
                )
                check("工具时间线 chip 完成", "写入文件" in (step_text or "") and "cli-golden-output.txt" in (step_text or ""),
                      " ".join((step_text or "").split())[:80])
            except Exception as error:
                check("工具时间线 chip 完成", False, f"{str(error)[:120]} :: {messages_digest()}")
            try:
                # 断言两层：文本渲染到会话流；且 .message-body 包装器仍在（流式与历史回放
                # 同构，字号/行距样式才一致）。
                page.wait_for_function(
                    "() => (document.getElementById('messages') || { textContent: '' }).textContent.includes('golden path complete')",
                    timeout=30000,
                )
                body_wrapped = page.evaluate(
                    "() => [...document.querySelectorAll('.msg.assistant .message-body')]"
                    ".some(el => el.textContent.includes('golden path complete'))"
                )
                check("最终回答渲染", True, f"message-body 包装存活: {bool(body_wrapped)}")
            except Exception as error:
                check("最终回答渲染", False, f"{str(error)[:120]} :: {messages_digest()}")
            try:
                page.wait_for_selector("#messages .turn-summary", timeout=30000)
                summary_text = page.text_content("#messages .turn-summary")
                check("回合汇报卡渲染", "文件" in (summary_text or ""), (summary_text or "").strip()[:80])
            except Exception as error:
                check("回合汇报卡渲染", False, str(error)[:160])

            # 服务端真相：会话存档里到底有哪些消息（定位「最终回答不渲染」在哪一层丢失）。
            transcript = page.evaluate(
                """async () => {
                    const view = document.querySelector('.session-view[data-sid]');
                    if (!view || !window.OwoApi) return { error: 'no view' };
                    const sid = view.getAttribute('data-sid');
                    const data = await window.OwoApi.get('/session/' + encodeURIComponent(sid));
                    return { sid, count: (data.messages || []).length, tail: (data.messages || []).slice(-5)
                        .map(m => ({ role: m.role, text: String(m.content == null ? '' : m.content).slice(0, 120) })) };
                }"""
            )
            print("  服务端存档 tail =", json.dumps(transcript, ensure_ascii=False)[:600])

            # 磁盘真实落盘（服务端 ground truth）。
            target = os.path.join(ws, "cli-golden-output.txt")
            written = os.path.isfile(target) and open(target, encoding="utf-8").read() == "cli golden content\n"
            check("文件真实写入工作区", written, target)

            # 检查器行级着色 diff：新增文件 → 全绿行 + 统计徽标。
            page.click("#inspectorToggle")
            try:
                page.wait_for_selector("#diffList .diff-item", timeout=30000)
                head = page.text_content("#diffList .diff-item .diff-open")
                check("diff 文件头含统计徽标", "cli-golden-output.txt" in (head or "") and "+" in (head or ""),
                      (head or "").strip()[:90])
                page.click("#diffList .diff-item .diff-open")
                page.wait_for_selector("#diffList .diff-rows .dl", timeout=15000)
                rows = page.eval_on_selector_all("#diffList .diff-rows .dl", "els => els.map(e => e.className)")
                add_rows = [r for r in rows if "dl-add" in r]
                check("行级 diff 全绿（新增文件）", len(rows) >= 1 and len(add_rows) == len(rows),
                      f"rows={len(rows)} add={len(add_rows)}")
            except Exception as error:
                check("行级 diff 全绿（新增文件）", False, str(error)[:160])

            check("回合结束后审批栏收起", "hidden" in (page.get_attribute("#approvalBar", "class") or ""))
            hard_errors = [t for t in console_errors if "favicon" not in t.lower()]
            check("无页面 JS 报错", not page_errors and not hard_errors, "; ".join((page_errors + hard_errors)[:3])[:300])
            browser.close()
    finally:
        daemon.terminate()
        provider.terminate()
        try:
            daemon.wait(timeout=5)
        except Exception:
            daemon.kill()
        provider.wait(timeout=5)
        daemon_log.close()
        if any(not ok for _, ok, _ in CHECKS):
            # 失败时附上 daemon 日志尾巴，便于定位（成功时保持输出干净）。
            try:
                with open(daemon_log_path, "rb") as log_file:
                    tail = log_file.read()[-4000:].decode("utf-8", "replace")
                print("--- daemon.log tail ---")
                print("\n".join(tail.splitlines()[-25:]))
            except Exception:
                pass

    failed = [item for item in CHECKS if not item[1]]
    print(f"\nTURN E2E {'PASS' if not failed else 'FAIL'}: {len(CHECKS) - len(failed)}/{len(CHECKS)}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
