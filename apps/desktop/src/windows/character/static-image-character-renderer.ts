import type { CharacterRendererDto } from '@parallel-world/contracts';
import type { CharacterRenderer } from './character-renderer';

type StaticRendererDto = Extract<CharacterRendererDto, { kind: 'static_image' }>;

type FetchResult = Pick<Response, 'ok' | 'status' | 'blob'>;
interface MaskContext {
  drawImage(bitmap: CanvasImageSource, dx: number, dy: number): void;
  getImageData(sx: number, sy: number, sw: number, sh: number): ImageData;
}
interface MaskCanvas {
  getContext(contextId: '2d'): MaskContext | null;
}

export interface SpeechReaction {
  react(turnId: number): boolean;
  reset(): void;
}

export interface StaticImageRendererDependencies {
  convertFileSrc(path: string): string;
  fetch(url: string): Promise<FetchResult>;
  createImageBitmap(blob: Blob): Promise<ImageBitmap>;
  createMaskCanvas(width: number, height: number): MaskCanvas;
  speechReaction?: SpeechReaction;
}

interface StaticFrame {
  readonly bitmap: ImageBitmap;
  readonly opaque: Uint8Array;
}

interface DrawGeometry {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

const ALPHA_THRESHOLD = 16;

function defaultMaskCanvas(width: number, height: number): MaskCanvas {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(width, height);
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  return canvas as MaskCanvas;
}

export function defaultStaticImageDependencies(
  convertFileSrc: (path: string) => string,
): StaticImageRendererDependencies {
  return {
    convertFileSrc,
    fetch: (url) => globalThis.fetch(url),
    createImageBitmap: (blob) => globalThis.createImageBitmap(blob),
    createMaskCanvas: defaultMaskCanvas,
  };
}

/** Full-frame static renderer with all-or-nothing expression preloading. */
export class StaticImageCharacterRenderer implements CharacterRenderer {
  readonly kind = 'static_image' as const;
  readonly #canvas: HTMLCanvasElement;
  readonly #context: CanvasRenderingContext2D;
  readonly #dependencies: StaticImageRendererDependencies;
  readonly #closed = new WeakSet<object>();
  #frames = new Map<string, StaticFrame>();
  #manifest: StaticRendererDto | null = null;
  #currentExpression: string | null = null;
  #cssWidth: number;
  #cssHeight: number;
  #dpr = 1;
  #geometry: DrawGeometry | null = null;
  #disposed = false;
  #loading = false;

  constructor(canvas: HTMLCanvasElement, dependencies: StaticImageRendererDependencies) {
    const context = canvas.getContext('2d');
    if (context === null) throw new Error('2D canvas context is unavailable');
    this.#canvas = canvas;
    this.#context = context;
    this.#dependencies = dependencies;
    this.#cssWidth = canvas.clientWidth || canvas.width || 1;
    this.#cssHeight = canvas.clientHeight || canvas.height || 1;
  }

  async load(renderer: CharacterRendererDto): Promise<void> {
    if (renderer.kind !== 'static_image') {
      throw new Error('StaticImageCharacterRenderer requires a static_image manifest');
    }
    this.#ensureActive();
    if (this.#loading || this.#manifest !== null) {
      throw new Error('StaticImageCharacterRenderer is already loaded');
    }
    this.#loading = true;
    const decoded: StaticFrame[] = [];
    try {
      const results = await Promise.allSettled(renderer.expressions.map(async (expression) => {
        const frame = await this.#decodeFrame(expression.image_path, renderer.width, renderer.height);
        decoded.push(frame);
        if (this.#disposed) this.#closeBitmap(frame.bitmap);
        return [expression.name, frame] as const;
      }));
      const failure = results.find(
        (result): result is PromiseRejectedResult => result.status === 'rejected',
      );
      if (failure !== undefined) throw failure.reason;
      if (this.#disposed) {
        for (const frame of decoded) this.#closeBitmap(frame.bitmap);
        return;
      }
      const entries = results.map((result) => {
        if (result.status !== 'fulfilled') {
          throw new Error('unreachable rejected static expression result');
        }
        return result.value;
      });
      const frames = new Map(entries);
      if (!frames.has(renderer.default_expression)) {
        throw new Error(`default expression is not decoded: ${renderer.default_expression}`);
      }
      this.#frames = frames;
      this.#manifest = renderer;
      this.#currentExpression = renderer.default_expression;
      this.#drawCurrent();
    } catch (error) {
      for (const frame of decoded) this.#closeBitmap(frame.bitmap);
      throw error;
    } finally {
      this.#loading = false;
    }
  }

  setExpression(name: string): boolean {
    if (this.#disposed || !this.#frames.has(name)) return false;
    this.#currentExpression = name;
    this.#drawCurrent();
    return true;
  }

  startMotion(_group: string): boolean {
    return false;
  }

  setAudioLevel(_level: number): void {}

  reactToSpeechStart(turnId: number): boolean {
    if (this.#disposed) return false;
    return this.#dependencies.speechReaction?.react(turnId) ?? false;
  }

  resetSpeechReaction(): void {
    this.#dependencies.speechReaction?.reset();
  }

  resize(width: number, height: number, dpr: number): void {
    if (this.#disposed) return;
    this.#cssWidth = Math.max(1, width);
    this.#cssHeight = Math.max(1, height);
    this.#dpr = Math.max(1, dpr);
    this.#canvas.width = Math.max(1, Math.round(this.#cssWidth * this.#dpr));
    this.#canvas.height = Math.max(1, Math.round(this.#cssHeight * this.#dpr));
    this.#drawCurrent();
  }

  hitTest(x: number, y: number): boolean {
    const geometry = this.#geometry;
    const manifest = this.#manifest;
    const frame = this.#currentExpression === null
      ? undefined
      : this.#frames.get(this.#currentExpression);
    if (this.#disposed || geometry === null || manifest === null || frame === undefined) return false;
    if (x < geometry.left || y < geometry.top || x >= geometry.left + geometry.width || y >= geometry.top + geometry.height) {
      return false;
    }
    const sourceX = Math.min(
      manifest.width - 1,
      Math.floor(((x - geometry.left) / geometry.width) * manifest.width),
    );
    const sourceY = Math.min(
      manifest.height - 1,
      Math.floor(((y - geometry.top) / geometry.height) * manifest.height),
    );
    return frame.opaque[sourceY * manifest.width + sourceX] === 1;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#dependencies.speechReaction?.reset();
    for (const frame of this.#frames.values()) this.#closeBitmap(frame.bitmap);
    this.#frames.clear();
    this.#manifest = null;
    this.#currentExpression = null;
    this.#geometry = null;
    this.#context.clearRect(0, 0, this.#cssWidth, this.#cssHeight);
  }

  async #decodeFrame(path: string, width: number, height: number): Promise<StaticFrame> {
    const url = this.#dependencies.convertFileSrc(path);
    const response = await this.#dependencies.fetch(url);
    if (!response.ok) throw new Error(`failed to fetch static expression (${response.status}): ${path}`);
    const bitmap = await this.#dependencies.createImageBitmap(await response.blob());
    if (bitmap.width !== width || bitmap.height !== height) {
      this.#closeBitmap(bitmap);
      throw new Error(`decoded expression dimensions ${bitmap.width}x${bitmap.height} do not match ${width}x${height}: ${path}`);
    }
    try {
      const maskCanvas = this.#dependencies.createMaskCanvas(width, height);
      const maskContext = maskCanvas.getContext('2d');
      if (maskContext === null) throw new Error('2D alpha-mask context is unavailable');
      maskContext.drawImage(bitmap, 0, 0);
      const rgba = maskContext.getImageData(0, 0, width, height).data;
      const opaque = new Uint8Array(width * height);
      for (let index = 0; index < opaque.length; index += 1) {
        opaque[index] = (rgba[index * 4 + 3] ?? 0) >= ALPHA_THRESHOLD ? 1 : 0;
      }
      return { bitmap, opaque };
    } catch (error) {
      this.#closeBitmap(bitmap);
      throw error;
    }
  }

  #drawCurrent(): void {
    const manifest = this.#manifest;
    const frame = this.#currentExpression === null
      ? undefined
      : this.#frames.get(this.#currentExpression);
    if (this.#disposed || manifest === null || frame === undefined) return;
    const scale = Math.min(this.#cssWidth / manifest.width, this.#cssHeight / manifest.height);
    const width = manifest.width * scale;
    const height = manifest.height * scale;
    const geometry = {
      left: (this.#cssWidth - width) / 2,
      top: this.#cssHeight - height,
      width,
      height,
    };
    this.#context.setTransform(this.#dpr, 0, 0, this.#dpr, 0, 0);
    this.#context.clearRect(0, 0, this.#cssWidth, this.#cssHeight);
    this.#context.drawImage(
      frame.bitmap,
      geometry.left,
      geometry.top,
      geometry.width,
      geometry.height,
    );
    this.#geometry = geometry;
  }

  #closeBitmap(bitmap: ImageBitmap): void {
    if (this.#closed.has(bitmap)) return;
    this.#closed.add(bitmap);
    bitmap.close();
  }

  #ensureActive(): void {
    if (this.#disposed) throw new Error('StaticImageCharacterRenderer is disposed');
  }
}

export type { StaticRendererDto };
