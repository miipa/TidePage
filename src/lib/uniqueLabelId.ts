// 设置页 label/控件 id 的唯一分配器（纯函数 + 模块级递增序号）。
// 为什么需要：旧实现每次 associateLabels 都从 1 重新计数，条件字段挂载后重跑会生成
// 重复 id（set-f1 与已有控件冲突），for/aria-labelledby 指错目标。
// 序号从 1000 起、循环跳过已占用 id，保证跨多次调用单调且不撞已有 DOM。

let seq = 1000;

/** 分配 `${prefix}${seq}`；isTaken 命中时继续递增，直到拿到未被占用的 id。 */
export function nextUniqueLabelId(prefix: string, isTaken: (id: string) => boolean): string {
  // 防御：isTaken 若恒 true 会死循环——最多尝试 100000 次后抛错（调用方在 DEV 可看到）。
  for (let i = 0; i < 100_000; i++) {
    seq += 1;
    const id = `${prefix}${seq}`;
    if (!isTaken(id)) return id;
  }
  throw new Error("[settings] nextUniqueLabelId exhausted");
}

/** 仅测试用：重置序号，保证用例可重复。 */
export function __resetUniqueLabelSeqForTests(): void {
  seq = 1000;
}
