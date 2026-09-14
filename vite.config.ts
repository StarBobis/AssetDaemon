import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes('node_modules')) return
          if (id.includes('/three/') || id.includes('\\three\\')) return 'three'
          if (id.includes('/element-plus/') || id.includes('\\element-plus\\')) return 'element-plus'
          if (id.includes('/@element-plus/') || id.includes('\\@element-plus\\')) return 'element-plus'
          if (id.includes('/@tauri-apps/') || id.includes('\\@tauri-apps\\')) return 'tauri'
          if (id.includes('/vue') || id.includes('\\vue')) return 'vue'
          return 'vendor'
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
