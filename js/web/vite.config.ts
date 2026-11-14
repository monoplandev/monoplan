 import { defineConfig } from "vite";
  import solid from "vite-plugin-solid";
  import wasm from "vite-plugin-wasm";

  export default defineConfig({
    plugins: [wasm(), solid()],
    build: {
      target: "es2022",     // explicit; TLA is es2022
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
