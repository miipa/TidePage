import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import pkg from "./package.json";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// 版本号单一数据源：从 package.json 读取，与 src-tauri/tauri.conf.json 的 version 保持一致，
// 以 __APP_VERSION__ 常量在构建期注入前端。

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [sveltekit()],

  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
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
      //    另外挡掉「点开头的临时目录 + *.tmp」：编辑工具会在源码目录里建临时目录再改名，
      //    watcher 抓到刚被 rename 掉的路径会 EBUSY，整个 dev server 会因此退出
      //    （beforeDevCommand 非零 → tauri dev 被拆）。
      ignored: ["**/src-tauri/**", "**/.*.tmpdir", "**/.*.tmpdir/**", "**/*.tmp"],
    },
  },
}));
