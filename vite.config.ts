import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    // Build/cache trees are not frontend sources. Watching staged directories can
    // also keep Windows directory handles open across a release rename.
    watch: {
      ignored: [
        "**/release-output/**",
        "**/.fluxdl-release-work/**",
        "**/release/**",
        "**/artifacts/**",
        "**/src-tauri/target/**",
      ],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: { target: "es2022", sourcemap: false },
  test: { environment: "jsdom", include: ["src/**/*.test.{ts,tsx}"] },
});
