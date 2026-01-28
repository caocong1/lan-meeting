import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import UnoCSS from "unocss/vite";

export default defineConfig({
  plugins: [solid(), UnoCSS()],

  // Vite options for Tauri
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    target: "esnext",
    minify: "esbuild",
    sourcemap: false,
  },

  // Env variables prefix
  envPrefix: ["VITE_", "TAURI_"],
});
