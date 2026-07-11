import { CubismFramework, Option } from '@framework/live2dcubismframework';
import { CubismMatrix44 } from '@framework/math/cubismmatrix44';
import { CubismWebGLOffscreenManager } from '@framework/rendering/cubismoffscreenmanager';
import { LAppModel } from './lappmodel';
import { LAppPal } from './lapppal';
import { LAppSubdelegate } from './lappsubdelegate';

type Context = { canvas: HTMLCanvasElement; gl: WebGLRenderingContext | WebGL2RenderingContext; model3JsonUrl: string };

let users = 0;
function startFramework(): void {
  if (!CubismFramework.isStarted()) CubismFramework.startUp(new Option());
  if (!CubismFramework.isInitialized()) CubismFramework.initialize();
  users += 1;
}

function stopFramework(): void {
  users = Math.max(0, users - 1);
  if (users === 0 && CubismFramework.isInitialized()) CubismFramework.dispose();
}

async function waitUntilDrawable(model: LAppModel): Promise<void> {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline) {
    const setting = model._modelSetting;
    const expectedTextures = setting?.getTextureCount() ?? -1;
    if (model.getModel() && model.getRenderer() && expectedTextures >= 0 &&
        model._textureCount >= expectedTextures) return;
    await new Promise<void>(resolve => setTimeout(resolve, 16));
  }
  throw new Error('Cubism model load timed out');
}

export const ParallelWorldCubismR5Bridge = {
  async createModelFromModel3({ canvas, gl, model3JsonUrl }: Context) {
    startFramework();
    let active = true;
    const subdelegate = new LAppSubdelegate();
    if (!subdelegate.initialize(canvas)) {
      stopFramework();
      throw new Error('Cubism WebGL initialization failed');
    }
    if (subdelegate.getGl() !== gl) {
      subdelegate.release();
      stopFramework();
      throw new Error('Cubism bridge acquired a different WebGL context');
    }
    const manager = subdelegate.getLive2DManager();
    const model = new LAppModel();
    model.setSubdelegate(subdelegate);
    manager._models.push(model);
    const url = new URL(model3JsonUrl, document.baseURI);
    const slash = url.pathname.lastIndexOf('/');
    model.loadAssets(url.href.slice(0, url.href.lastIndexOf('/') + 1), decodeURIComponent(url.pathname.slice(slash + 1)));
    try {
      // Do not abort an in-flight official Sample load. Its Image callbacks
      // are not cancellable; resolving only after every texture upload makes
      // React StrictMode's deferred disposal safe.
      await waitUntilDrawable(model);
    } catch (error) {
      active = false;
      subdelegate.release();
      stopFramework();
      throw error;
    }
    return {
      update(deltaSeconds: number) {
        if (!active) return;
        LAppPal.deltaTime = Math.max(0, deltaSeconds);
        model.update();
      },
      draw() {
        if (!active) return;
        const gl = subdelegate.getGl();
        gl.viewport(0, 0, canvas.width, canvas.height);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        CubismWebGLOffscreenManager.getInstance().beginFrameProcess(gl);
        const projection = new CubismMatrix44();
        const width = Math.max(1, canvas.width);
        const height = Math.max(1, canvas.height);
        if (model.getModel().getCanvasWidth() > 1 && width < height) {
          model.getModelMatrix().setWidth(2);
          projection.scale(1, width / height);
        } else projection.scale(height / width, 1);
        model.draw(projection);
        CubismWebGLOffscreenManager.getInstance().endFrameProcess(gl);
      },
      playMotion(group: string, index = 0) { model.startMotion(group, index, 3); },
      setExpression(id: string) { model.setExpression(id); },
      resize(width: number, height: number) {
        canvas.width = width; canvas.height = height;
        subdelegate.getGl().viewport(0, 0, width, height);
        model.setRenderTargetSize(width, height);
      },
      dispose() {
        if (!active) return;
        active = false;
        manager._models.length = 0;
        model.release();
        subdelegate.release();
        stopFramework();
      }
    };
  }
};

Object.defineProperty(globalThis, 'ParallelWorldCubismR5Bridge', {
  value: ParallelWorldCubismR5Bridge, configurable: true
});
