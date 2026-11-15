import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { PageBoundary } from "@/components/PageBoundary";
import { applyChatWidth, readChatWidth } from "@/lib/chat-width";
import { loadLocale } from "@/lib/i18n";
import { settleInterfaceMode } from "@/lib/new-look";
import { wearSkin, THEME_SKINS } from "@/lib/skins";
import { followOwOTheme } from "./theme";
import "./entry.css";

settleInterfaceMode();
if (!followOwOTheme()) {
  document.documentElement.dataset.owoTheme = "light";
  wearSkin(THEME_SKINS.light);
}
applyChatWidth(readChatWidth());

async function start() {
  const root = createRoot(document.getElementById("root")!);
  try {
    const [{ OwOBotWorkbench }] = await Promise.all([import("./Workbench"), loadLocale("zh")]);
    root.render(<StrictMode><PageBoundary><OwOBotWorkbench /></PageBoundary></StrictMode>);
  } catch {
    root.render(<p role="alert">OwOBot 加载失败，请从桌面工作台重新打开。</p>);
  }
}

void start();
