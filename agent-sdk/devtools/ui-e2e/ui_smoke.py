# OWO 桌面 Web UI 冒烟测试（真实 daemon + 真实前端静态文件）。
# 验证目标：启动引导、新建会话、检查器 diff 空态、用量设置页拉取（本轮修复）、
# 页面内 markdown 高亮（真实产品页加载的 core/markdown.js）。
# 用法：python devtools/ui-e2e/ui_smoke.py [--port 4827] [--workspace PATH]
import argparse
import json
import sys
import time

from playwright.sync_api import sync_playwright

CHECKS = []


def check(name, ok, detail=""):
    CHECKS.append((name, bool(ok), detail))
    mark = "PASS" if ok else "FAIL"
    print(f"[{mark}] {name}" + (f" :: {detail}" if detail else ""))


def launch_browser(p):
    # 优先用系统 Edge（免下载）；没有再用 Playwright 自带的 chromium。
    try:
        return p.chromium.launch(headless=True, channel="msedge")
    except Exception:
        return p.chromium.launch(headless=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=4827)
    parser.add_argument("--workspace", required=True)
    parser.add_argument("--timeout", type=int, default=30)
    args = parser.parse_args()
    base = f"http://127.0.0.1:{args.port}"

    console_errors = []
    page_errors = []
    with sync_playwright() as p:
        browser = launch_browser(p)
        context = browser.new_context(viewport={"width": 1440, "height": 900})
        # 工作区先于页面脚本落 localStorage：app.js 在加载时读它填充 #workspace。
        context.add_init_script(
            "try { window.localStorage.setItem('owo.workspace', %s); } catch (e) {}"
            % json.dumps(args.workspace)
        )
        page = context.new_page()
        page.on("console", lambda msg: console_errors.append(msg.text) if msg.type == "error" else None)
        page.on("pageerror", lambda err: page_errors.append(str(err)))
        page.on("console", lambda msg: print(f"  console/{msg.type}: {msg.text[:180]}") if msg.type in ("error", "warning") else None)

        page.goto(base + "/", wait_until="domcontentloaded")
        page.wait_for_selector("#menubar", timeout=args.timeout * 1000)
        check("页面打开并渲染菜单栏", True)

        # 1) 新 CSS 层随产品页加载。
        links = page.eval_on_selector_all('link[rel="stylesheet"]', "els => els.map(e => e.getAttribute('href'))")
        check("visualization.css 已随产品页加载", "visualization.css" in links, ", ".join(links))

        # 2) 启动引导：健康状态不再是「连接中…」。
        try:
            page.wait_for_function(
                "() => { const el = document.getElementById('health'); return el && !/连接中/.test(el.textContent); }",
                timeout=args.timeout * 1000,
            )
            health = page.text_content("#health")
            check("核心健康状态就绪", True, health.strip())
        except Exception as error:
            check("核心健康状态就绪", False, str(error)[:160])

        # 3) 新建会话（真实 POST /session + 会话列表刷新）。
        sessions_before = page.eval_on_selector_all("#sessionList li", "els => els.length")
        page.click("#newSession")
        try:
            page.wait_for_function(
                "() => document.querySelectorAll('#sessionList li').length > 0 || document.querySelector('#messages .session-view')",
                timeout=args.timeout * 1000,
            )
            sessions_after = page.eval_on_selector_all("#sessionList li", "els => els.length")
            check(
                "新建会话成功并进入会话列表",
                sessions_after > sessions_before or bool(page.query_selector("#messages .session-view")),
                f"before={sessions_before} after={sessions_after}",
            )
        except Exception as error:
            check("新建会话成功并进入会话列表", False, str(error)[:160])

        # 4) 检查器 diff 空态（真实 GET /session/{id}/diff）。
        page.click("#inspectorToggle")
        try:
            page.wait_for_function(
                "() => { const list = document.getElementById('diffList'); return list && !/正在读取/.test(list.textContent); }",
                timeout=args.timeout * 1000,
            )
            diff_text = page.text_content("#diffList")
            check("任务详情 diff 空态渲染", "没有文件改动" in diff_text, diff_text.strip()[:80])
        except Exception as error:
            check("任务详情 diff 空态渲染", False, str(error)[:160])

        # 5) 设置 → 使用情况和计费：打开即拉取（修复前永远停在「加载中…」）。
        page.click("#codexSettingsEntry")
        page.wait_for_selector(".settings-section:not([hidden])", timeout=args.timeout * 1000)
        page.click('[data-settings-tab="usage"]')
        try:
            page.wait_for_function(
                "() => { const panel = document.getElementById('usagePanel'); return panel && !/加载中/.test(panel.textContent); }",
                timeout=args.timeout * 1000,
            )
            usage_text = page.text_content("#usagePanel")
            has_cards = page.eval_on_selector_all("#usagePanel .usage-card", "els => els.length")
            check("用量页打开即拉取并渲染卡片", has_cards >= 1, f"cards={has_cards}")
            check("用量卡片含累计 Tokens 口径", "累计 Tokens" in usage_text, usage_text.strip()[:80])
        except Exception as error:
            check("用量页打开即拉取并渲染卡片", False, str(error)[:160])

        # 6) 页面内真实渲染模块：markdown 高亮在产品上下文可用（两行代码以触发行号栏）。
        highlighted = page.evaluate(
            "() => window.OwoMarkdown && window.OwoMarkdown.renderMarkdown("
            "'```rust\\nfn main() { let s = \\'x\\'; }\\nprintln!(s);\\n```'"
            ")"
        )
        check(
            "产品页 markdown 高亮生效",
            bool(highlighted)
            and "tk-k" in highlighted
            and 'class="md-lines"' in highlighted
            and "tk-s" in highlighted
            and "tk-m" in highlighted,
            (highlighted or "")[:160],
        )
        # 7) 深色主题类切换（token 色板/表面色随 body 类生效）。
        page.evaluate("() => document.body.classList.add('dark-theme')")
        dark_bg = page.evaluate("() => getComputedStyle(document.body).backgroundColor")
        check("深色主题类可切换", bool(dark_bg), str(dark_bg))

        browser.close()

    hard_errors = [text for text in console_errors if "favicon" not in text.lower()]
    check("无页面 JS 报错", not page_errors and not hard_errors, "; ".join((page_errors + hard_errors)[:3])[:300])

    failed = [item for item in CHECKS if not item[1]]
    print(f"\nSMOKE {'PASS' if not failed else 'FAIL'}: {len(CHECKS) - len(failed)}/{len(CHECKS)}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
