 import { fileURLToPath } from "node:url";
 import { defineConfig } from "vite";
  import solid from "vite-plugin-solid";
  import wasm from "vite-plugin-wasm";

  export default defineConfig({
    plugins: [wasm(), solid()],
    build: {
      target: "es2022",     // explicit; TLA is es2022
      rollupOptions: {
        // Two entries from one source tree: the browser client, and the
        // macOS app's window (`spec/mac-window-plan.md`), which runs
        // the same UI over a mirror doc. The server embeds `dist/`
        // whole and serves `index.html`; `mirror.html` is only ever
        // loaded by the app from its bundle.
        input: {
          main: fileURLToPath(new URL("./index.html", import.meta.url)),
          mirror: fileURLToPath(new URL("./mirror.html", import.meta.url)),
        },
      },
    },
    server: {
      port: 5176,
      allowedHosts: ["macbook.yokoso.golf"],
      proxy: {
        "/api": {
          target: process.env.VITE_API_TARGET ?? "http://localhost:8000",
          changeOrigin: true,
          ws: true,
        },
        "/admin": {
          target: process.env.VITE_API_TARGET ?? "http://localhost:8000",
          changeOrigin: true,
        },
      },
    },
    optimizeDeps: {
      exclude: ["@monoplan/core"],
    },
  });
