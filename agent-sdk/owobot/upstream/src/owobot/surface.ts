/** Managed frontend served by OwO alongside the desktop workbench. */
export function isOwOPage(): boolean {
  return globalThis.location?.pathname === "/owobot/";
}

export function collaborationPath(path: string): string {
  return isOwOPage() && path.startsWith("/api/") ? "/owobot" + path : path;
}
