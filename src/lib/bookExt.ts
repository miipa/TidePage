/** 书籍可识别扩展名（小写，无点）。前端唯一来源，替代各组件内重复的本地常量。 */
export const BOOK_EXTENSIONS: readonly string[] = ["txt", "epub", "mobi", "azw3", "prc"];

/** 由 BOOK_EXTENSIONS 派生、锚定结尾、大小写不敏感的扩展名剥离正则。 */
const BOOK_EXT_STRIP_RE = new RegExp(`\\.(${BOOK_EXTENSIONS.join("|")})$`, "i");

/** 去掉文件名结尾的书籍扩展名（大小写不敏感）；无匹配时原样返回。 */
export function stripBookExt(name: string): string {
  return name.replace(BOOK_EXT_STRIP_RE, "");
}
