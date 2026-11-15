import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

const path = (part: string) => fileURLToPath(new URL(part, import.meta.url));
const { version } = JSON.parse(readFileSync(path("./package.json"), "utf8"));

export default defineConfig({
  root: path("./src/owobot/"),
  base: "/owobot/",
  publicDir: false,
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path("./src/") } },
  define: { __APP_VERSION__: JSON.stringify(version) },
  build: {
    license: { fileName: "licenses/frontend-dependencies.txt" },
    outDir: path("./dist-owobot/"),
    emptyOutDir: true,
  },
});
