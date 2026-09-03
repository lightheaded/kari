import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFile } from "node:fs/promises";
import { basename, join } from "node:path";
import type { Plugin } from "vite";

/** Browser preview without Tauri: serve `<dir>/<name>.json` at `/dev/<name>.json` in dev only.
 *  `dir` is `KARI_FIXTURES` or `fixtures/`. The files live outside public/ so a build never bundles a board.
 *  `fixtures/board.json` is a real local board (gitignored). `docs/demo/` holds the dummy board for screenshots. */
function devFixtures(): Plugin {
  const dir = process.env.KARI_FIXTURES || "fixtures";
  return {
    name: "kari-dev-fixtures",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/dev", async (req, res) => {
        const name = basename(req.url?.split("?")[0] ?? "");
        if (!/^[a-z0-9-]+\.json$/.test(name)) {
          res.statusCode = 404;
          res.end("not a fixture");
          return;
        }
        try {
          res.setHeader("content-type", "application/json");
          res.end(await readFile(join(dir, name)));
        } catch {
          res.statusCode = 404;
          res.end(`no ${join(dir, name)}`);
        }
      });
    },
  };
}

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), devFixtures()],
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
