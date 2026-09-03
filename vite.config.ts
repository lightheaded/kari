import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFile } from "node:fs/promises";
import type { Plugin } from "vite";

/** Browser preview without Tauri: serve fixtures/dev-board.json at /dev-board.json in dev only.
 *  The file lives outside public/ so a build never bundles a real board. */
function devBoard(): Plugin {
  return {
    name: "kari-dev-board",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/dev-board.json", async (_req, res) => {
        try {
          res.setHeader("content-type", "application/json");
          res.end(await readFile("fixtures/dev-board.json"));
        } catch {
          res.statusCode = 404;
          res.end("no fixtures/dev-board.json");
        }
      });
    },
  };
}

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), devBoard()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: { target: "safari15", minify: !process.env.TAURI_ENV_DEBUG, sourcemap: !!process.env.TAURI_ENV_DEBUG },
});
