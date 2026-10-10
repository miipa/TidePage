// See https://svelte.dev/docs/kit/types#app.d.ts
declare global {
  /** 构建期由 vite.config.js 注入的应用版本号（来自 package.json）。 */
  const __APP_VERSION__: string;
}

export {};
