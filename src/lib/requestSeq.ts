/**
 * F9：请求序号（纯类）。只有最新一次请求可以写状态，旧响应直接丢弃。
 * 面板组件用它覆盖 StandalonePanel/NotesPanel/PdfSearchPanel 的竞态。
 */
export class RequestSeq {
  private seq = 0;

  next(): number {
    this.seq += 1;
    return this.seq;
  }

  isCurrent(token: number): boolean {
    return token === this.seq;
  }
}
