import { createCharacterController, CubismRuntimeAdapter, Live2DError, type CharacterManifest } from '@parallel-world/live2d-runtime';

const markManifest: CharacterManifest = {
  schemaVersion: 1, id: 'mark', model3: 'Mark.model3.json', motions: { Idle: [0, 1, 2, 3, 4, 5] }, expressions: {},
};
const epsilonExpressions = Object.fromEntries(['Angry', 'Blushing', 'f01', 'f02', 'Normal', 'Sad', 'Smile', 'Surprised'].map(id => [id, `expressions/${id}.exp3.json`]));
const epsilonManifest: CharacterManifest = {
  schemaVersion: 1, id: 'epsilon-free', model3: 'Epsilon_free.model3.json',
  motions: { Idle: [0], FlickUp: [0, 1], Flick: [0, 1], Tap: [0, 1, 2, 3], Flick3: [0, 1], FlickDown: [0, 1], Shake: [0, 1] },
  expressions: epsilonExpressions,
};

export function getLive2DCharacterManifest(modelId: string): CharacterManifest {
  if (modelId === 'mark') return markManifest;
  if (modelId === 'epsilon-free') return epsilonManifest;
  throw new Live2DError('invalid-model-source');
}

export function createLive2DCharacterDevController() {
  if (!globalThis.ParallelWorldCubismR5Bridge) throw new Live2DError('framework-unavailable');
  if (!globalThis.Live2DCubismCore) throw new Live2DError('core-unavailable');
  let frames = 0;
  const adapter = new CubismRuntimeAdapter({ onFrame: (canvas, gl) => {
    frames += 1; canvas.dataset.live2dFrame = String(frames);
    if (frames === 1 || frames % 30 === 0) {
      const pixels = new Uint8Array(canvas.width * canvas.height * 4);
      gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      let alpha = 0; for (let i = 3; i < pixels.length; i += 4) if (pixels[i]) alpha += 1;
      canvas.dataset.live2dAlphaPixels = String(alpha);
      if (alpha > 0) canvas.dataset.live2dState = 'ready';
    }
  }});
  return createCharacterController({ adapter, loadManifest: async source => getLive2DCharacterManifest(source.modelId) });
}

declare global {
  var ParallelWorldCubismR5Bridge: import('@parallel-world/live2d-runtime').CubismR5Bridge | undefined;
  var Live2DCubismCore: unknown;
}
