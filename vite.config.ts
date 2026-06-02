import { defineConfig } from "vite";

// The frontend root is the repo root: index.html lives here, app code in src/,
// and the shared graph fixtures in fixtures/ (imported directly during dev).
// Port 1420 + strictPort is the Tauri convention so the desktop shell (A8) can
// point at this dev server without renegotiating the port.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
