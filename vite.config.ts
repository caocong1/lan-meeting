import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import UnoCSS from "unocss/vite";
import { resolve } from "path";

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
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        viewer: resolve(__dirname, "viewer.html"),
      },
    },
  },

  // Env variables prefix
  envPrefix: ["VITE_", "TAURI_"],
});
