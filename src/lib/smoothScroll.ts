export interface EdgeEvent {
  edge: -1 | 0 | 1;
  carry: number;
}

export class SmoothScroller {
  private target = 0;
  private current = 0;
  private max = 0;
  private over = 0;
  private pending = 0;
  private raf = 0;
  private apply: (renderPos: number) => void;
  private onStop?: (ratio: number) => void;
  private stopTimer: ReturnType<typeof setTimeout> | undefined;

  bounce = true;
  strength = 0.5;
  threshold = 140;

  constructor(apply: (renderPos: number) => void) {
    this.apply = apply;
  }

  setMax(m: number) {
    this.max = m;
    this.target = Math.max(0, Math.min(this.target, m));
    this.current = Math.max(0, Math.min(this.current, m));
  }

  get pos() {
    return this.current;
  }

  get targetPos() {
    return this.target;
  }

  ratio() {
    return this.max > 0 ? Math.max(0, Math.min(1, this.current / this.max)) : 0;
  }

  reset() {
    this.over = 0;
    this.pending = 0;
  }

  jumpTo(pos: number, immediate = true) {
    this.target = Math.max(0, Math.min(pos, this.max));
    this.over = 0;
    this.pending = 0;
    if (immediate) {
      this.current = this.target;
      this.apply(this.current);
    } else {
      this.ensureLoop();
    }
  }

  wheelBy(delta: number): EdgeEvent {
    let to = this.target + delta;
    let over = 0;
    if (to < 0) {
      over = to;
      to = 0;
    } else if (to > this.max) {
      over = to - this.max;
      to = this.max;
    }
    this.target = to;

    let edge: -1 | 0 | 1 = 0;
    let carry = 0;
    if (over !== 0) {
      const sign = over > 0 ? 1 : -1;
      if (this.pending === 0 || Math.sign(this.pending) === sign) {
        this.pending += over;
      } else {
        this.pending = over;
      }
      if (!this.bounce) {
        edge = sign as -1 | 1;
        carry = over;
        this.pending = 0;
        this.over = 0;
      } else {
        const damp = 1.0 / (1 + this.strength * 1.1);
        const cap = this.threshold * 0.55;
        this.over = Math.max(-cap, Math.min(cap, this.over + over * damp));
        this.pending += over * 0.6;
        const limit = this.threshold;
        if (Math.abs(this.pending) >= limit) {
          edge = sign as -1 | 1;
          carry = Math.max(0, Math.abs(this.pending) - limit) / 0.6;
          this.pending = 0;
        }
      }
    } else {
      this.pending = 0;
    }

    this.ensureLoop();
    return { edge, carry };
  }

  private ensureLoop() {
    if (!this.raf) this.raf = requestAnimationFrame(this.tick);
  }

  private render() {
    this.apply(this.current + this.over);
  }

  private tick = () => {
    this.raf = 0;
    let cont = false;
    const d = this.target - this.current;
    if (Math.abs(d) > 0.6) {
      this.current += d * 0.22;
      cont = true;
    } else {
      this.current = this.target;
    }
    if (Math.abs(this.over) > 0.6) {
      this.over *= 0.76;
      cont = true;
    } else {
      this.over = 0;
    }
    this.render();
    if (cont) {
      this.raf = requestAnimationFrame(this.tick);
    } else {
      clearTimeout(this.stopTimer);
      this.stopTimer = setTimeout(() => this.onStop?.(this.ratio()), 200);
    }
  };

  onSettled(cb: (ratio: number) => void) {
    this.onStop = cb;
  }

  destroy() {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    clearTimeout(this.stopTimer);
  }
}
