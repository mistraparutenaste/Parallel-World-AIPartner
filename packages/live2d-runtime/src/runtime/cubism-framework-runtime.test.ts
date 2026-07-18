import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ModelSource } from './cubism-runtime';

const SOURCE: ModelSource = {
  modelUrl: 'asset:model.model3.json',
  resolveResource: (path) => `asset:${path}`,
};

function deferred<T = void>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function stubCubismCore(): void {
  vi.stubGlobal('Live2DCubismCore', {
    ColorBlendType_Normal: 0,
    ColorBlendType_AddGlow: 1,
    ColorBlendType_Add: 2,
    ColorBlendType_Darken: 3,
    ColorBlendType_Multiply: 4,
    ColorBlendType_ColorBurn: 5,
    ColorBlendType_LinearBurn: 6,
    ColorBlendType_Lighten: 7,
    ColorBlendType_Screen: 8,
    ColorBlendType_ColorDodge: 9,
    ColorBlendType_Overlay: 10,
    ColorBlendType_SoftLight: 11,
    ColorBlendType_HardLight: 12,
    ColorBlendType_LinearLight: 13,
    ColorBlendType_Hue: 14,
    ColorBlendType_Color: 15,
    ColorBlendType_AddCompatible: 16,
    ColorBlendType_MultiplyCompatible: 17,
  });
}

function canvasFor(gl: WebGL2RenderingContext): HTMLCanvasElement {
  return {
    width: 320,
    height: 480,
    getContext: vi.fn(() => gl),
  } as unknown as HTMLCanvasElement;
}

async function runtimeModules() {
  stubCubismCore();
  const [runtime, model, framework, shaders] = await Promise.all([
    import('./cubism-framework-runtime'),
    import('../model/character-model'),
    import('../../vendor/framework/src/live2dcubismframework'),
    import('../../vendor/framework/src/rendering/cubismshader_webgl'),
  ]);
  vi.spyOn(framework.CubismFramework, 'isStarted').mockReturnValue(true);
  vi.spyOn(framework.CubismFramework, 'isInitialized').mockReturnValue(true);
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  return { ...runtime, ...model, ...shaders };
}

describe('CubismFrameworkRuntime resource ownership', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('releases the real CharacterModel instance when loading fails', async () => {
    const { CharacterModel, CubismFrameworkRuntime } = await runtimeModules();
    const failure = new Error('shader load failed');
    const load = vi.spyOn(CharacterModel.prototype, 'load').mockRejectedValue(failure);
    const release = vi.spyOn(CharacterModel.prototype, 'release').mockImplementation(() => {});
    const runtime = new CubismFrameworkRuntime({ shaderPath: '/live2d/shaders/' });
    await runtime.start(canvasFor({} as WebGL2RenderingContext));

    await expect(runtime.loadModel(SOURCE)).rejects.toBe(failure);

    expect(load).toHaveBeenCalledOnce();
    expect(release).toHaveBeenCalledOnce();
    expect(release.mock.instances[0]).toBe(load.mock.instances[0]);
    runtime.stop();
  });

  it('drops each stopped canvas context before a replacement starts', async () => {
    const { CubismFrameworkRuntime, CubismShaderManager_WebGL } = await runtimeModules();
    const manager = CubismShaderManager_WebGL.getInstance();
    const firstGl = {} as WebGL2RenderingContext;
    const secondGl = {} as WebGL2RenderingContext;
    const runtime = new CubismFrameworkRuntime({ shaderPath: '/live2d/shaders/' });

    try {
      await runtime.start(canvasFor(firstGl));
      manager.setGlContext(firstGl);
      runtime.stop();
      expect(manager.getShader(firstGl)).toBeUndefined();

      await runtime.start(canvasFor(secondGl));
      manager.setGlContext(secondGl);
      expect(manager.getShader(firstGl)).toBeUndefined();
      expect(manager.getShader(secondGl)).toBeDefined();

      runtime.stop();
      expect(manager.getShader(secondGl)).toBeUndefined();
    } finally {
      CubismShaderManager_WebGL.deleteInstance();
    }
  });

  it('does not resurrect a model that registers its context after stop', async () => {
    const {
      CharacterModel,
      CubismFrameworkRuntime,
      CubismShaderManager_WebGL,
    } = await runtimeModules();
    CubismShaderManager_WebGL.deleteInstance();
    const manager = CubismShaderManager_WebGL.getInstance();
    const staleLoad = deferred();
    const load = vi.spyOn(CharacterModel.prototype, 'load')
      .mockImplementationOnce(() => staleLoad.promise)
      .mockResolvedValue(undefined);
    const release = vi.spyOn(CharacterModel.prototype, 'release').mockImplementation(() => {});
    const staleGl = {} as WebGL2RenderingContext;
    const currentGl = {} as WebGL2RenderingContext;
    const runtime = new CubismFrameworkRuntime({ shaderPath: '/live2d/shaders/' });

    try {
      await runtime.start(canvasFor(staleGl));
      const staleResult = runtime.loadModel(SOURCE);
      await vi.waitFor(() => expect(load).toHaveBeenCalledOnce());
      runtime.stop();

      // The old renderer starts after stop(), which used to make the
      // context release a no-op and allowed this load to revive #model.
      manager.setGlContext(staleGl);
      staleLoad.resolve();

      await expect(staleResult).rejects.toThrow(
        'runtime changed while the model was loading',
      );
      expect(release).toHaveBeenCalledOnce();
      expect(release.mock.instances[0]).toBe(load.mock.instances[0]);
      expect(manager.getShader(staleGl)).toBeUndefined();

      await runtime.start(canvasFor(currentGl));
      await expect(runtime.loadModel(SOURCE)).resolves.toBeDefined();
      expect(load).toHaveBeenCalledTimes(2);
      expect(release).toHaveBeenCalledOnce();
    } finally {
      runtime.stop();
      CubismShaderManager_WebGL.deleteInstance();
    }
  });

  it('releases a late context when a stale model load rejects', async () => {
    const {
      CharacterModel,
      CubismFrameworkRuntime,
      CubismShaderManager_WebGL,
    } = await runtimeModules();
    CubismShaderManager_WebGL.deleteInstance();
    const manager = CubismShaderManager_WebGL.getInstance();
    const staleLoad = deferred();
    const failure = new Error('late shader failure');
    const load = vi.spyOn(CharacterModel.prototype, 'load')
      .mockImplementation(() => staleLoad.promise);
    const release = vi.spyOn(CharacterModel.prototype, 'release').mockImplementation(() => {});
    const staleGl = {} as WebGL2RenderingContext;
    const runtime = new CubismFrameworkRuntime({ shaderPath: '/live2d/shaders/' });

    try {
      await runtime.start(canvasFor(staleGl));
      const staleResult = runtime.loadModel(SOURCE);
      await vi.waitFor(() => expect(load).toHaveBeenCalledOnce());
      runtime.stop();
      manager.setGlContext(staleGl);
      staleLoad.reject(failure);

      await expect(staleResult).rejects.toBe(failure);
      expect(release).toHaveBeenCalledOnce();
      expect(manager.getShader(staleGl)).toBeUndefined();
    } finally {
      runtime.stop();
      CubismShaderManager_WebGL.deleteInstance();
    }
  });
});
