import { Live2DError } from '@parallel-world/live2d-runtime';
import { CharacterCanvas } from '../../features/character/CharacterCanvas';
import { isTauri } from '@tauri-apps/api/core';
import { tauriCharacterPresentationTransport } from '../../features/character/tauriCharacterPresentation';
import '../../shared/styles/global.css';

const modelSource = import.meta.env.DEV
  ? { modelId: 'mark', manifestUrl: '__live2d_dev__/models/live2d-mark/character.json' } as const
  : { modelId: 'fallback', manifestUrl: 'fallback/character.json' } as const;
const createController = async () => {
  if (!import.meta.env.DEV) throw new Live2DError('framework-unavailable');
  const dev = await import('./Live2DCharacterDev');
  return dev.createLive2DCharacterDevController();
};
const resolvePresentationModel = (modelId: string) => import.meta.env.DEV
  ? { modelId, manifestUrl: modelId === 'epsilon-free' ? '__live2d_dev__/models/live2d-epsilon/epsilon_free/runtime/character.json' : '__live2d_dev__/models/live2d-mark/character.json' }
  : { modelId: 'fallback', manifestUrl: 'fallback/character.json' };

export function CharacterWindow() {
  return <main className="character-stage">
    <div className="character-stage__drag" data-tauri-drag-region aria-hidden="true">
      {Array.from({ length: 9 }, (_, index) => <i key={index} />)}
    </div>
    <div className="character-stage__depth" aria-hidden="true" />
    <CharacterCanvas controllerFactory={createController} modelSource={modelSource} presentationTransport={isTauri() ? tauriCharacterPresentationTransport : undefined} resolvePresentationModel={resolvePresentationModel} />
  </main>;
}
