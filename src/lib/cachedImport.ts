// 动态 import 的「成功才缓存」包装。
// 为什么需要：`let p; p ??= import(...)` 在 import 失败（网络/构建产物缺失/磁盘错误）后会把
// rejected promise 永久缓存，之后所有调用都拿到同一个错误，用户只能重启应用才能恢复。
// 这里成功缓存、失败清空缓存并 rethrow；并发调用共享同一个 in-flight promise，不会重复加载。

export type ImportLoader<T> = () => Promise<T>;

/**
 * 返回一个与 loader 同签名的加载函数：首次调用发起 import 并缓存 promise；
 * 成功保持缓存；失败（含同步抛错）清空缓存，下一次调用可以重新加载。
 */
export function cachedImport<T>(loader: ImportLoader<T>): () => Promise<T> {
  let cached: Promise<T> | null = null;
  let token = 0;
  return () => {
    if (cached) return cached;
    let p: Promise<T>;
    try {
      p = loader();
    } catch (e) {
      // 同步抛错：不写缓存，直接让调用方看到错误；下次调用仍会重试。
      throw e;
    }
    const myToken = ++token;
    const wrapped = p.catch((e) => {
      // 只清本次失败对应的缓存；期间若已被新加载替换则保持新值。
      if (myToken === token) cached = null;
      throw e;
    });
    cached = wrapped;
    return wrapped;
  };
}
