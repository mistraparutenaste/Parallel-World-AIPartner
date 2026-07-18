import { afterEach, describe, expect, it, vi } from 'vitest';

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

function successfulShaderResponse(): Response {
  return {
    ok: true,
    status: 200,
    text: vi.fn(async () => 'void main() {}'),
  } as unknown as Response;
}

function shaderGl(
  compileSucceeds: () => boolean,
  linkSucceeds: () => boolean = () => true,
) {
  let nextId = 0;
  const createProgram = vi.fn(
    () => ({ kind: 'program', id: ++nextId }) as unknown as WebGLProgram,
  );
  const createShader = vi.fn(
    () => ({ kind: 'shader', id: ++nextId }) as unknown as WebGLShader,
  );
  const deleteShader = vi.fn();
  const deleteProgram = vi.fn();
  const gl = {
    VERTEX_SHADER: 0x8b31,
    FRAGMENT_SHADER: 0x8b30,
    COMPILE_STATUS: 0x8b81,
    LINK_STATUS: 0x8b82,
    createProgram,
    createShader,
    shaderSource: vi.fn(),
    compileShader: vi.fn(),
    getShaderParameter: vi.fn(() => compileSucceeds()),
    getShaderInfoLog: vi.fn(() => 'compile boom'),
    deleteShader,
    attachShader: vi.fn(),
    linkProgram: vi.fn(),
    getProgramParameter: vi.fn(() => linkSucceeds()),
    getProgramInfoLog: vi.fn(() => 'link boom'),
    deleteProgram,
  } as unknown as WebGL2RenderingContext;
  return { gl, createProgram, createShader, deleteShader, deleteProgram };
}

describe('Cubism shader loading', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('rejects renderer initialization when a shader resource cannot be fetched', async () => {
    stubCubismCore();
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('', { status: 503 })),
    );
    const { CubismShader_WebGL } = await import(
      '../../vendor/framework/src/rendering/cubismshader_webgl'
    );
    const shader = new CubismShader_WebGL();
    shader.setShaderPath('/live2d/shaders/');

    await expect(shader.generateShaders()).rejects.toThrow(
      'failed to fetch shader /live2d/shaders/vertshadersrc.vert: 503',
    );
    expect(shader._isShaderLoaded).toBe(false);
  });

  it('removes a released context and prevents its pending load from registering shaders', async () => {
    stubCubismCore();
    const { CubismShaderManager_WebGL } = await import(
      '../../vendor/framework/src/rendering/cubismshader_webgl'
    );
    CubismShaderManager_WebGL.deleteInstance();
    const manager = CubismShaderManager_WebGL.getInstance();
    const gl = {} as WebGL2RenderingContext;
    manager.setGlContext(gl);
    const shader = manager.getShader(gl);
    const registerShader = vi.spyOn(shader, 'registerShader').mockImplementation(() => {});
    const registerBlendShader = vi.spyOn(shader, 'registerBlendShader').mockImplementation(() => {});
    let resolveFetch!: (response: Response) => void;
    const pendingFetch = new Promise<Response>((resolve) => {
      resolveFetch = resolve;
    });
    vi.stubGlobal('fetch', vi.fn(() => pendingFetch));

    try {
      const loading = shader.generateShaders();
      await vi.waitFor(() => expect(fetch).toHaveBeenCalled());
      const releaseGlContext = Reflect.get(manager, 'releaseGlContext') as
        | ((context: WebGLRenderingContext) => void)
        | undefined;
      releaseGlContext?.call(manager, gl);
      resolveFetch(successfulShaderResponse());

      await expect(loading).rejects.toThrow('shader loading was cancelled');
      expect(registerShader).not.toHaveBeenCalled();
      expect(registerBlendShader).not.toHaveBeenCalled();
      expect(manager.getShader(gl)).toBeUndefined();
    } finally {
      CubismShaderManager_WebGL.deleteInstance();
    }
  });

  it('keeps compile failures unhealthy, cleans resources, and permits retry', async () => {
    stubCubismCore();
    const { CubismShader_WebGL } = await import(
      '../../vendor/framework/src/rendering/cubismshader_webgl'
    );
    let compileSucceeds = false;
    const { gl, deleteShader, deleteProgram } = shaderGl(
      () => compileSucceeds,
    );
    const shader = new CubismShader_WebGL();
    shader.setGl(gl);
    vi.stubGlobal('fetch', vi.fn(async () => successfulShaderResponse()));
    vi.spyOn(shader, 'registerShader').mockImplementation(() => {
      shader._shaderSets[0].shaderProgram = shader.loadShaderProgram(
        'vertex source',
        'fragment source',
      );
    });
    vi.spyOn(shader, 'registerBlendShader').mockImplementation(() => {});

    await expect(shader.generateShaders()).rejects.toThrow(
      'shader compilation failed: compile boom',
    );
    expect(shader._isShaderLoaded).toBe(false);
    expect(deleteShader).toHaveBeenCalledOnce();
    expect(deleteProgram).toHaveBeenCalledOnce();

    compileSucceeds = true;
    await expect(shader.generateShaders()).resolves.toBeUndefined();
    expect(shader._isShaderLoaded).toBe(true);
    expect(fetch).toHaveBeenCalledTimes(26);
    shader.release();
  });

  it('throws on link failure and deletes both shaders and the program', async () => {
    stubCubismCore();
    const { CubismShader_WebGL } = await import(
      '../../vendor/framework/src/rendering/cubismshader_webgl'
    );
    const { gl, deleteShader, deleteProgram } = shaderGl(
      () => true,
      () => false,
    );
    const shader = new CubismShader_WebGL();
    shader.setGl(gl);

    expect(() => shader.loadShaderProgram('vertex', 'fragment')).toThrow(
      'shader link failed: link boom',
    );
    expect(deleteShader).toHaveBeenCalledTimes(2);
    expect(deleteProgram).toHaveBeenCalledOnce();
  });
});
