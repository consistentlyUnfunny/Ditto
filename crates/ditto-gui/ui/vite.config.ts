import { defineConfig } from "vite";

export default defineConfig({
  // Tauri serves the built files from disk, so assets must be referenced
  // relatively rather than from an absolute root.
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "esnext",
  },
  server: {
    port: 1420,
    strictPort: true,
  },
});
