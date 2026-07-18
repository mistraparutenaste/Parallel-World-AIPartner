/**
 * CubismRuntime implementation on top of the vendored framework, the
 * Cubism Core global and a WebGL canvas. This module is the only
 * place that touches WebGL and the SDK at runtime.
 */

import {
  CubismFramework,
  LogLevel,
  Option,
} from '../../vendor/framework/src/live2dcubismframework';
import { CubismMatrix44 } from '../../vendor/framework/src/math/cubismmatrix44';
import {
  CubismShaderManager_WebGL,
} from '../../vendor/framework/src/rendering/cubismshader_webgl';
import { CharacterModel } from '../model/character-model';
import type { CubismRuntime, ModelHandle, ModelSource } from './cubism-runtime';
import { FrameTimer } from './frame-timer';

export type CubismFrameworkRuntimeOptions = {
  /** Directory URL containing the framework's WebGL shader files. */
  shaderPath: string;
  /** Minimum alpha (0-255) that counts as a model hit. */
  hitAlphaThreshold?: number;
};

const DEFAULT_HIT_ALPHA_THRESHOLD = 16;

type GL = WebGLRenderingContext | WebGL2RenderingContext;

function coreIsLoaded(): boolean {
  return typeof (globalThis as { Live2DCubismCore?: unknown })
    .Live2DCubismCore !== 'undefined';
}

export class CubismFrameworkRuntime implements CubismRuntime {
  #options: Required<CubismFrameworkRuntimeOptions>;
  #canvas: HTMLCanvasElement | null = null;
  #gl: GL | null = null;
  #model: CharacterModel | null = null;
  #timer: FrameTimer | null = null;
  #frameHandle: number | null = null;
  #lifecycleGeneration = 0;

  constructor(options: CubismFrameworkRuntimeOptions) {
    this.#options = {
      hitAlphaThreshold: DEFAULT_HIT_ALPHA_THRESHOLD,
      ...options,
    };
  }

  async start(canvas: HTMLCanvasElement): Promise<void> {
    if (!coreIsLoaded()) {
      throw new Error(
        'Live2D Cubism Core is not loaded (missing live2dcubismcore script)',
      );
    }
    const attributes: WebGLContextAttributes = {
      alpha: true,
      premultipliedAlpha: true,
      // Keeps the frame readable for alpha hit-testing (click-through).
      preserveDrawingBuffer: true,
    };
    const gl =
      canvas.getContext('webgl2', attributes) ??
      canvas.getContext('webgl', attributes);
    if (gl == null) {
      throw new Error('WebGL is unavailable');
    }
    if (this.#gl !== null || this.#canvas !== null) {
      this.stop();
    }
    this.#lifecycleGeneration++;
    this.#canvas = canvas;
    this.#gl = gl;

    if (!CubismFramework.isStarted()) {
      const option = new Option();
      option.logFunction = (message: string) => {
        console.log(message);
      };
      option.loggingLevel = LogLevel.LogLevel_Warning;
      CubismFramework.startUp(option);
    }
    if (!CubismFramework.isInitialized()) {
      CubismFramework.initialize();
    }

    this.#timer = new FrameTimer(performance.now());
    this.#scheduleFrame();
  }

  async loadModel(source: ModelSource): Promise<ModelHandle> {
    const gl = this.#gl;
    const canvas = this.#canvas;
    if (gl == null || canvas == null) {
      throw new Error('runtime is not started');
    }
    const generation = this.#lifecycleGeneration;
    const model = new CharacterModel(gl, canvas, this.#options.shaderPath);
    try {
      await model.load(source);
    } catch (error) {
      const stale = this.#loadIsStale(generation, gl, canvas);
      try {
        this.#releaseLoadResources(model, gl, stale);
      } finally {
        throw error;
      }
    }
    if (this.#loadIsStale(generation, gl, canvas)) {
      const staleError = new Error('runtime changed while the model was loading');
      try {
        this.#releaseLoadResources(model, gl, true);
      } finally {
        throw staleError;
      }
    }

    this.#model?.release();
    this.#model = model;

    const runtime = this;
    return {
      get expressions(): readonly string[] {
        return model.expressionNames;
      },
      get motionGroups(): ReadonlyMap<string, number> {
        return model.motionGroupCounts;
      },
      setExpression: (name: string) => model.setExpressionByName(name),
      startMotion: (group: string, index?: number) =>
        model.startMotionIn(group, index),
      setLipSyncValue: (value: number) => model.setLipSyncValue(value),
      hitTest: (x: number, y: number) => runtime.#hitTestAlpha(x, y),
      release: () => {
        if (runtime.#model === model) {
          runtime.#model = null;
        }
        model.release();
      },
    };
  }

  resize(_width: number, _height: number): void {
    // The render loop reads canvas.width/height every frame; nothing
    // else must be recomputed because the projection follows it.
  }

  stop(): void {
    this.#lifecycleGeneration++;
    if (this.#frameHandle !== null) {
      cancelAnimationFrame(this.#frameHandle);
      this.#frameHandle = null;
    }
    const gl = this.#gl;
    try {
      this.#model?.release();
    } finally {
      this.#model = null;
      if (gl !== null) {
        CubismShaderManager_WebGL.getInstance().releaseGlContext(gl);
      }
      // CubismFramework stays initialized: it is process-global and may
      // be reused by a later runtime instance in the same page.
      this.#gl = null;
      this.#canvas = null;
    }
  }

  #scheduleFrame(): void {
    this.#frameHandle = requestAnimationFrame(() => {
      this.#renderFrame();
      if (this.#gl != null) {
        this.#scheduleFrame();
      }
    });
  }

  #loadIsStale(
    generation: number,
    gl: GL,
    canvas: HTMLCanvasElement,
  ): boolean {
    return generation !== this.#lifecycleGeneration
      || this.#gl !== gl
      || this.#canvas !== canvas;
  }

  #releaseLoadResources(
    model: CharacterModel,
    gl: GL,
    releaseContext: boolean,
  ): void {
    try {
      model.release();
    } finally {
      if (releaseContext) {
        CubismShaderManager_WebGL.getInstance().releaseGlContext(gl);
      }
    }
  }

  #renderFrame(): void {
    const gl = this.#gl;
    const canvas = this.#canvas;
    if (gl == null || canvas == null) {
      return;
    }
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(0.0, 0.0, 0.0, 0.0);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.clear(gl.COLOR_BUFFER_BIT);

    const model = this.#model;
    const timer = this.#timer;
    if (model == null || timer == null) {
      return;
    }
    model.updateFrame(timer.tick(performance.now()));
    model.draw(this.#projectionFor(canvas, model));
  }

  #projectionFor(
    canvas: HTMLCanvasElement,
    model: CharacterModel,
  ): CubismMatrix44 {
    // Mirrors the official sample's LAppView projection: fit the
    // model's logical canvas into the window while preserving aspect.
    const projection = new CubismMatrix44();
    const cubismModel = model.getModel();
    if (
      cubismModel.getCanvasWidth() > 1.0 &&
      canvas.width < canvas.height
    ) {
      model.getModelMatrix().setWidth(2.0);
      projection.scale(1.0, canvas.width / canvas.height);
    } else {
      projection.scale(canvas.height / canvas.width, 1.0);
    }
    return projection;
  }

  #hitTestAlpha(cssX: number, cssY: number): boolean {
    const gl = this.#gl;
    const canvas = this.#canvas;
    if (gl == null || canvas == null || this.#model == null) {
      return false;
    }
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      return false;
    }
    const px = Math.floor((cssX / rect.width) * canvas.width);
    const py = canvas.height - 1 - Math.floor((cssY / rect.height) * canvas.height);
    if (px < 0 || py < 0 || px >= canvas.width || py >= canvas.height) {
      return false;
    }
    const pixel = new Uint8Array(4);
    gl.readPixels(px, py, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return pixel[3] >= this.#options.hitAlphaThreshold;
  }
}
