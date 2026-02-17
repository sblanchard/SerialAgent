import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { resolve } from "path";

// Detect Tauri dev mode via env var set by `cargo tauri dev`
const isTauri = !!process.env.TAURI_ENV_PLATFORM;

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { "@": resolve(__dirname, "src") },
  },

  // Prevent Vite from obscuring Rust errors in the Tauri CLI output
  clearScreen: false,

  server: {
    port: 5173,
    // Tauri needs the dev server to be accessible
    host: isTauri ? "0.0.0.0" : "localhost",
    strictPort: true,
    proxy: {
      "/v1": "http://localhost:3210",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Tauri uses Chromium on Windows and WebKit on macOS/Linux
    target: isTauri ? "chrome105" : "modules",
  },
});
