/**
 * Abstraction over the Cubism SDK so the controller (and its tests)
 * never touch WebGL, the Core global or the vendored framework.
 */

/** A loaded, renderable character model. */
export interface ModelHandle {
  /** Expression names declared by the model, in manifest order. */
  readonly expressions: readonly string[];
  /** Motion group names with the number of motions in each group. */
  readonly motionGroups: ReadonlyMap<string, number>;
  /** Applies a named expression. Returns false for unknown names. */
  setExpression(name: string): boolean;
  /**
   * Starts a motion from the given group (random index when omitted).
   * Returns false for unknown groups.
   */
  startMotion(group: string, index?: number): boolean;
  /**
   * True when the model pixel at canvas-relative CSS coordinates is
   * opaque enough to count as a hit.
   */
  hitTest(x: number, y: number): boolean;
  /** Releases model resources. */
  release(): void;
}

/** Runtime service that owns the render surface and loop. */
export interface CubismRuntime {
  /**
   * Initializes the Cubism core and the render surface, then starts
   * the render loop. Rejects when the runtime cannot start (missing
   * core script, WebGL unavailable).
   */
  start(canvas: HTMLCanvasElement): Promise<void>;
  /** Loads a model and makes it the rendered model. */
  loadModel(modelUrl: string): Promise<ModelHandle>;
  /** Propagates a resize of the canvas backing store (physical px). */
  resize(width: number, height: number): void;
  /** Stops the render loop and releases runtime resources. */
  stop(): void;
}
