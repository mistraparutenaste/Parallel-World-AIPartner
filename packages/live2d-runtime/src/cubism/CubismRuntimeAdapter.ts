import { Live2DError, type CharacterModelHandle, type CharacterModelSource, type CharacterRuntimeAdapter } from "../contracts.js";
import type { CharacterManifest } from "../manifest/CharacterManifest.js";

export interface CubismR5ModelContext {
  canvas: HTMLCanvasElement;
  gl: WebGLRenderingContext | WebGL2RenderingContext;
  model3JsonUrl: string;
}

/**
 * Boundary implemented by the local Cubism 5 R5 development loader.
 * It must use CubismModelSettingJson and the Framework scheduler; the runtime
 * package deliberately does not copy proprietary Core or Sample source.
 */
export interface CubismR5Bridge {
  createModelFromModel3(context: CubismR5ModelContext): Promise<CharacterModelHandle>;
}

export interface CubismRuntimeAdapterOptions {
  globalObject?: Record<string, unknown>;
  bridge?: CubismR5Bridge;
  baseUrl?: string;
  onFrame?: (canvas: HTMLCanvasElement, gl: WebGLRenderingContext | WebGL2RenderingContext) => void;
}

function assertSelfModel3Url(value: string, baseUrl: string): string {
  let url: URL;
  try { url = new URL(value, baseUrl); }
  catch (cause) { throw new Live2DError("invalid-model-source", "invalid model3 URL", { cause }); }
  const base = new URL(baseUrl);
  if ((url.protocol !== "http:" && url.protocol !== "https:") || url.origin !== base.origin || !url.pathname.endsWith(".model3.json")) {
    throw new Live2DError("invalid-model-source", "model3 URL must be a same-origin HTTP(S) resource");
  }
  return url.href;
}

export class CubismRuntimeAdapter implements CharacterRuntimeAdapter {
  readonly #globalObject: Record<string, unknown>;
  readonly #bridge?: CubismR5Bridge;
  readonly #baseUrl: string;
  readonly #onFrame?: CubismRuntimeAdapterOptions["onFrame"];
  #canvas?: HTMLCanvasElement;

  constructor(options: CubismRuntimeAdapterOptions) {
    this.#globalObject = options.globalObject ?? globalThis as Record<string, unknown>;
    this.#bridge = options.bridge ?? this.#globalObject.ParallelWorldCubismR5Bridge as CubismR5Bridge | undefined;
    this.#baseUrl = options.baseUrl ?? (globalThis.document?.baseURI || "http://localhost/");
    this.#onFrame = options.onFrame;
  }

  mount(canvas: HTMLCanvasElement): void { this.#canvas = canvas; }

  async loadModel(source: CharacterModelSource, manifest: CharacterManifest): Promise<CharacterModelHandle> {
    if (!this.#canvas) throw new Live2DError("not-mounted");
    const manifestUrl = new URL(source.manifestUrl, this.#baseUrl);
    const model3Url = new URL(manifest.model3, manifestUrl).href;
    return this.createModel(this.#canvas, model3Url);
  }

  dispose(): void { this.#canvas = undefined; }

  async createModel(canvas: HTMLCanvasElement, model3JsonUrl: string): Promise<CharacterModelHandle> {
    if (!this.#globalObject.Live2DCubismCore) throw new Live2DError("core-unavailable", "Cubism Core global is unavailable");
    const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
    if (!gl) throw new Live2DError("webgl-unavailable", "WebGL context is unavailable");
    const safeUrl = assertSelfModel3Url(model3JsonUrl, this.#baseUrl);
    if (!this.#bridge?.createModelFromModel3) throw new Live2DError("framework-unavailable", "Cubism R5 Framework bridge is unavailable");
    try {
      const handle = await this.#bridge.createModelFromModel3({ canvas, gl, model3JsonUrl: safeUrl });
      if (!this.#onFrame) return handle;
      return { ...handle, draw: () => { handle.draw(); this.#onFrame?.(canvas, gl); } };
    } catch (cause) {
      if (cause instanceof Live2DError) throw cause;
      throw new Live2DError("model-load-failed", "Cubism Framework failed to create model", { cause });
    }
  }
}
