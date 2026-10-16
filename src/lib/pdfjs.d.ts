// pdf.js 的 legacy 构建没有随包提供 .mjs 对应的类型入口；且本项目 checkJs 打开，
// 直接让 TS 去检查整个 pdf.mjs 会非常慢。这里把 legacy 入口映射到官方类型声明
// （运行时 API 与 build/pdf.mjs 一致）。
declare module "pdfjs-dist/legacy/build/pdf.mjs" {
  export * from "pdfjs-dist";
}

// Vite 的 ?url 导入：返回构建产物里的资源 URL（pdf.js 的 worker 脚本按 URL 加载）。
declare module "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url" {
  const url: string;
  export default url;
}
