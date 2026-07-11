import type { CubismRuntime, ModelHandle } from '../runtime/cubism-runtime';

/** Observable lifecycle states of the controller. */
export type Live2DControllerState =
  | 'idle'
  | 'starting'
  | 'ready'
  | 'model-loaded'
  | 'unavailable'
  | 'disposed';

export type StateChangeListener = (state: Live2DControllerState) => void;

const NO_MOTION_GROUPS: ReadonlyMap<string, number> = new Map();

/**
 * React-independent facade over the Live2D runtime.
 *
 * UI layers talk to this class only; the WebGL / Cubism SDK details
 * live behind [`CubismRuntime`]. Every state transition is reported
 * through the optional listener so views can mirror the lifecycle.
 */
export class Live2DController {
  #runtime: CubismRuntime;
  #onStateChange: StateChangeListener | undefined;
  #state: Live2DControllerState = 'idle';
  #model: ModelHandle | null = null;
  #canvas: HTMLCanvasElement | null = null;

  constructor(runtime: CubismRuntime, onStateChange?: StateChangeListener) {
    this.#runtime = runtime;
    this.#onStateChange = onStateChange;
  }

  get state(): Live2DControllerState {
    return this.#state;
  }

  get expressions(): readonly string[] {
    return this.#model?.expressions ?? [];
  }

  get motionGroups(): ReadonlyMap<string, number> {
    return this.#model?.motionGroups ?? NO_MOTION_GROUPS;
  }

  /**
   * Starts the runtime on the given canvas. On failure the controller
   * settles in `unavailable` instead of throwing, so callers can show
   * the degraded state described in the design spec.
   */
  async attach(canvas: HTMLCanvasElement): Promise<void> {
    if (this.#state === 'disposed') {
      throw new Error('Live2DController is disposed');
    }
    if (this.#state !== 'idle') {
      throw new Error(`attach() called in state ${this.#state}`);
    }
    this.#canvas = canvas;
    this.#setState('starting');
    try {
      await this.#runtime.start(canvas);
      this.#setState('ready');
    } catch {
      this.#setState('unavailable');
    }
  }

  /**
   * Loads a model. Rejects (and settles in `unavailable`) when the
   * model cannot be loaded.
   */
  async loadModel(modelUrl: string): Promise<void> {
    if (this.#state !== 'ready' && this.#state !== 'model-loaded') {
      throw new Error(`loadModel() called while not ready (${this.#state})`);
    }
    try {
      const model = await this.#runtime.loadModel(modelUrl);
      this.#model?.release();
      this.#model = model;
      this.#setState('model-loaded');
    } catch (error) {
      this.#setState('unavailable');
      throw error;
    }
  }

  /** Applies a named expression. False when no model is loaded. */
  setExpression(name: string): boolean {
    return this.#model?.setExpression(name) ?? false;
  }

  /** Starts a motion from a group. False when no model is loaded. */
  startMotion(group: string, index?: number): boolean {
    return this.#model?.startMotion(group, index) ?? false;
  }

  /** Hit-tests canvas-relative CSS coordinates against the model. */
  hitTest(x: number, y: number): boolean {
    return this.#model?.hitTest(x, y) ?? false;
  }

  /**
   * Resizes the canvas backing store to CSS size x device pixel ratio
   * and informs the runtime.
   */
  resize(cssWidth: number, cssHeight: number, devicePixelRatio: number): void {
    if (!this.#canvas) {
      return;
    }
    const width = Math.max(1, Math.round(cssWidth * devicePixelRatio));
    const height = Math.max(1, Math.round(cssHeight * devicePixelRatio));
    this.#canvas.width = width;
    this.#canvas.height = height;
    this.#runtime.resize(width, height);
  }

  /** Releases the model and stops the runtime. Idempotent. */
  dispose(): void {
    if (this.#state === 'disposed') {
      return;
    }
    this.#model?.release();
    this.#model = null;
    if (this.#state !== 'idle') {
      this.#runtime.stop();
    }
    this.#setState('disposed');
  }

  #setState(state: Live2DControllerState): void {
    this.#state = state;
    this.#onStateChange?.(state);
  }
}
