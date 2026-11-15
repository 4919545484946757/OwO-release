/** Managed frontend served by OwO alongside the desktop workbench. */
export function isOwOPage(): boolean {
  return globalThis.location?.pathname === "/owobot/";
}

export function collaborationPath(path: string): string {
  if (!isOwOPage() || !path.startsWith("/api/")) return path;
  const group = new URLSearchParams(location.search).get("group");
  if (!group) return "/owobot" + path;
  const url = new URL("/owobot" + path, location.origin);
  url.searchParams.set("owoProject", group);
  return url.pathname + url.search;
}
