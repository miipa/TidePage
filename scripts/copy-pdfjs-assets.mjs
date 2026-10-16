// 把 pdf.js 的 CMap 与标准字体拷进 static/pdfjs/，运行时按 URL 加载。
// 为什么必须拷：CMap 缺失会让中日韩 PDF 的字符映射失败（文字变方块），标准字体缺失会让
// 未内嵌字体的 PDF 版式错乱。两者都是静态数据文件，只能跟随前端产物一起发布。
// predev / prebuild 各跑一次；node_modules 里没有 pdfjs-dist 时只警告，不让构建失败。
import { cpSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "node_modules", "pdfjs-dist");

if (!existsSync(src)) {
  console.warn("[pdfjs] 未找到 node_modules/pdfjs-dist，跳过 CMap/标准字体拷贝");
  process.exit(0);
}

for (const name of ["cmaps", "standard_fonts"]) {
  const from = join(src, name);
  const to = join(root, "static", "pdfjs", name);
  if (!existsSync(from)) {
    console.warn(`[pdfjs] 缺少目录 ${from}，跳过`);
    continue;
  }
  rmSync(to, { recursive: true, force: true });
  mkdirSync(dirname(to), { recursive: true });
  cpSync(from, to, { recursive: true });
  console.log(`[pdfjs] ${name} -> static/pdfjs/${name}`);
}
