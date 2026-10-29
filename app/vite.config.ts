import react from "@vitejs/plugin-react";
import tailwind from "@tailwindcss/vite";
import { defineConfig } from "vitest/config";

// Tauri loads the built files (or this server in development).
export default defineConfig({
  plugins: [react(), tailwind()],
  clearScreen: false,
  // Tauri builds into src-tauri\target; Vite must not watch those files (Windows locks them while they build).
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", sourcemap: false },
  test: { environment: "happy-dom", include: ["tests/**/*.test.ts", "tests/**/*.test.tsx"] },
});
