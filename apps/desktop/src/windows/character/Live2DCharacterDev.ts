import { createCharacterController, CubismRuntimeAdapter, type CharacterManifest } from '@parallel-world/live2d-runtime';

const manifest: CharacterManifest = {
  schemaVersion: 1, id: 'mark', model3: 'Mark.model3.json', motions: { Idle: [0] }, expressions: {},
};

export function mountLive2DCharacterDev(canvas: HTMLCanvasElement): () => void {
  if (!globalThis.ParallelWorldCubismR5Bridge || !globalThis.Live2DCubismCore) return () => undefined;
  canvas.dataset.live2dState = 'loading';
  let frames = 0;
  const adapter = new CubismRuntimeAdapter({ onFrame: (_canvas, gl) => {
    frames += 1; canvas.dataset.live2dFrame = String(frames);
    if (frames === 1 || frames % 30 === 0) {
      const pixels = new Uint8Array(canvas.width * canvas.height * 4);
      gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      let alpha = 0; for (let i = 3; i < pixels.length; i += 4) if (pixels[i]) alpha += 1;
      canvas.dataset.live2dAlphaPixels = String(alpha);
      if (alpha > 0) canvas.dataset.live2dState = 'ready';
    }
  }});
  const controller = createCharacterController({ adapter, loadManifest: async () => manifest });
  let active = true;
  void controller.mount(canvas).then(() => {
    controller.resize({ cssWidth: canvas.clientWidth, cssHeight: canvas.clientHeight, devicePixelRatio: devicePixelRatio || 1 });
    return controller.loadModel({ modelId: 'mark', manifestUrl: '__live2d_dev__/models/live2d-mark/character.json' });
  }).catch(error => {
    if (!active) return;
    canvas.dataset.live2dState = 'error'; console.error('Live2D controller failed', error);
  });
  return () => { active = false; controller.dispose(); };
}

declare global {
  var ParallelWorldCubismR5Bridge: import('@parallel-world/live2d-runtime').CubismR5Bridge | undefined;
  var Live2DCubismCore: unknown;
}
