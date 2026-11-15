import { THEME_SKINS, wearSkin } from "../lib/skins";

export function followOwOTheme(page: Window = window): boolean {
  if (page.parent === page || !/^\/owobot(?:\/|$)/.test(page.location.pathname)) return false;

  let host: Document;
  try {
    host = page.parent.document;
  } catch {
    return false;
  }
  if (!host.body) return false;

  const root = page.document.documentElement;
  const apply = () => {
    const theme = host.body.classList.contains("dark-theme") ? "dark" : "light";
    if (root.dataset.owoTheme === theme) return;
    root.dataset.owoTheme = theme;
    wearSkin(THEME_SKINS[theme]);
  };
  apply();
  const observer = new MutationObserver(apply);
  observer.observe(host.body, { attributes: true, attributeFilter: ["class"] });
  page.addEventListener("pagehide", (event) => {
    if (!event.persisted) observer.disconnect();
  });
  return true;
}
