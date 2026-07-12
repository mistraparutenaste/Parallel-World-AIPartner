/**
 * Abstraction over the Cubism SDK so the controller (and its tests)
 * never touch WebGL, the Core global or the vendored framework.
 */

/**
 * Where a model lives and how to reach its resources.
 *
 * URL-space resolution is impossible for Tauri asset URLs (the whole
 * filesystem path is one encoded segment), so the embedder supplies a
 * resolver that maps model3.json-relative paths to fetchable URLs.
 */
export interface ModelSource {
  /** Fetchable URL of the model3.json itself. */
  modelUrl: string;
  /** Maps a path relative to the model3.json to a fetchable URL. */
  resolveResource: (relativePath: string) => string;
}

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
   * Sets the mouth-open value (0..1) computed from the playing audio
   * (Live2Dリップシンク).
   */
  setLipSyncValue(value: number): void;
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
  loadModel(source: ModelSource): Promise<ModelHandle>;
  /** Propagates a resize of the canvas backing store (physical px). */
  resize(width: number, height: number): void;
  /** Stops the render loop and releases runtime resources. */
  stop(): void;
}
