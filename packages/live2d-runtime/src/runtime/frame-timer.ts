/** Maximum delta so a suspended tab does not fast-forward motions. */
const MAX_DELTA_SECONDS = 0.1;

/** Per-frame delta time source with clamping. */
export class FrameTimer {
  #lastMilliseconds: number;

  constructor(nowMilliseconds: number) {
    this.#lastMilliseconds = nowMilliseconds;
  }

  /** Returns the clamped elapsed seconds since the previous tick. */
  tick(nowMilliseconds: number): number {
    const delta = (nowMilliseconds - this.#lastMilliseconds) / 1000;
    this.#lastMilliseconds = nowMilliseconds;
    if (delta < 0) {
      return 0;
    }
    return Math.min(delta, MAX_DELTA_SECONDS);
  }
}
