import type { CharacterManifest } from "./manifest/CharacterManifest.js";

export type Live2DErrorCode =
  | "not-mounted"
  | "mount-in-progress"
  | "mount-failed"
  | "not-ready"
  | "invalid-model-source"
  | "invalid-manifest"
  | "fetch-failed"
  | "model-load-failed"
  | "unknown-motion"
  | "unknown-expression"
  | "motion-failed"
  | "expression-failed"
  | "invalid-viewport"
  | "core-unavailable"
  | "webgl-unavailable"
  | "framework-unavailable"
  | "superseded"
  | "disposed";

export class Live2DError extends Error {
  readonly code: Live2DErrorCode;
  readonly cause?: unknown;

  constructor(code: Live2DErrorCode, message: string = code, options?: { cause?: unknown }) {
    super(message);
    this.name = "Live2DError";
    this.code = code;
    this.cause = options?.cause;
  }
}

export type CharacterRuntimeStatus =
  | { kind: "idle" }
  | { kind: "loading"; modelId: string }
  | { kind: "ready"; modelId: string }
  | { kind: "failed"; modelId: string; code: Live2DErrorCode };

export interface CharacterModelSource {
  modelId: string;
  manifestUrl: string;
}

export interface CharacterViewport {
  cssWidth: number;
  cssHeight: number;
  devicePixelRatio: number;
}

export interface CharacterModelHandle {
  update(deltaSeconds: number): void;
  draw(): void;
  playMotion(group: string, index?: number): void | Promise<void>;
  setExpression(id: string): void | Promise<void>;
  resize(physicalWidth: number, physicalHeight: number): void;
  dispose(): void;
}

export interface CharacterRuntimeAdapter {
  mount(canvas: HTMLCanvasElement): void | Promise<void>;
  loadModel(source: CharacterModelSource, manifest: CharacterManifest): Promise<CharacterModelHandle>;
  dispose(): void;
}

export interface CharacterRuntimeEnvironment {
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(id: number): void;
  addResizeListener(listener: () => void): () => void;
}

export type CharacterCleanupPhase =
  | "cancel-animation-frame"
  | "remove-resize-listener"
  | "dispose-model"
  | "dispose-adapter";

export interface CharacterController {
  mount(canvas: HTMLCanvasElement): Promise<void>;
  loadModel(source: CharacterModelSource): Promise<void>;
  playMotion(group: string, index?: number): Promise<void>;
  setExpression(id: string): Promise<void>;
  resize(viewport: CharacterViewport): void;
  dispose(): void;
  subscribe(listener: (status: CharacterRuntimeStatus) => void): () => void;
}

export type CharacterManifestLoader = (source: CharacterModelSource) => Promise<CharacterManifest>;
