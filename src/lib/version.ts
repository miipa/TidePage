/** 语义化版本比较：a 比 b 新返回 true。忽略 `v` 前缀与预发布(-)/构建(+)后缀,只比点分数字核心。 */
export function versionNewer(a: string, b: string): boolean {
  const parse = (s: string) =>
    s
      .replace(/^v/i, "")
      .split(/[-+]/)[0] // 截断预发布 / 构建元数据后缀
      .split(".")
      .map((x) => parseInt(x, 10) || 0);
  const av = parse(a);
  const bv = parse(b);
  for (let i = 0; i < Math.max(av.length, bv.length); i++) {
    const x = av[i] ?? 0;
    const y = bv[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}
