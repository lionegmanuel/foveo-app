import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Convenciones del template oficial de Tauri: puerto fijo 1420 (referenciado
// desde tauri.conf.json devUrl), HMR sobre el mismo puerto, e ignorar
// src-tauri/ en el watcher para no reiniciar Vite en cada `cargo build`.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
