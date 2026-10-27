/**
 * F7：可取消的节流定时器（纯类）。
 * windowSnap 的 scheduleEvaluate/schedulePreview 原来 setTimeout 句柄丢失、stopSnap 无法清理；
 * TimerGate 保存句柄，stop 后不再调度、已排队回调也不执行。
 */
export class TimerGate {
  private handle: ReturnType<typeof setTimeout> | null = null;
  private _stopped = false;

  get stopped(): boolean {
    return this._stopped;
  }

  schedule(fn: () => void, ms: number): void {
    if (this._stopped) return;
    this.cancel();
    this.handle = setTimeout(() => {
      this.handle = null;
      if (this._stopped) return;
      fn();
    }, ms);
  }

  cancel(): void {
    if (this.handle !== null) {
      clearTimeout(this.handle);
      this.handle = null;
    }
  }

  stop(): void {
    this._stopped = true;
    this.cancel();
  }
}
