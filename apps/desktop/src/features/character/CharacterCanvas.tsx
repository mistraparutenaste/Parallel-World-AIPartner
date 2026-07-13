import { useRef } from 'react';
import type { CharacterModelSource } from '@parallel-world/live2d-runtime';
import { StatusBadge } from '../../shared/components/StatusBadge';
import { useCharacterController, type CharacterControllerFactory } from './useCharacterController';
import type { CharacterModelSourceResolver, CharacterPresentationTransport } from './characterPresentation';

export interface CharacterCanvasPresentation { canvasLabel: string; loading: string; ready: string; failed: string }
const defaultPresentation: CharacterCanvasPresentation = { canvasLabel: 'Live2Dキャラクター', loading: '読み込み中', ready: '表示中', failed: '代替表示' };
export interface CharacterCanvasProps { controllerFactory: CharacterControllerFactory; modelSource: CharacterModelSource; presentation?: CharacterCanvasPresentation; presentationTransport?: CharacterPresentationTransport; resolvePresentationModel?: CharacterModelSourceResolver }

export function CharacterCanvas({ controllerFactory, modelSource, presentation = defaultPresentation, presentationTransport, resolvePresentationModel }: CharacterCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { status, errorCode } = useCharacterController(canvasRef, controllerFactory, modelSource, presentationTransport, resolvePresentationModel);
  const state = status.kind === 'idle' ? 'loading' : status.kind;
  const statusText = state === 'ready' ? presentation.ready : state === 'failed' ? presentation.failed : presentation.loading;
  return <>
    <canvas ref={canvasRef} className="character-stage__live2d" aria-label={presentation.canvasLabel} data-live2d-state={state} data-live2d-error-code={errorCode} />
    <svg data-testid="character-silhouette" className="character-stage__silhouette" aria-hidden="true" viewBox="0 0 280 600" fill="none">
      <path className="character-stage__aura" d="M140 38c-49 0-82 38-82 91 0 29 11 53 30 70-9 28-20 49-38 66 15 1 29-3 40-12-5 20-14 38-29 52 15 2 29-2 41-12-11 46-35 88-51 130-13 35-19 78-22 129h202c-3-51-9-94-22-129-16-42-40-84-51-130 12 10 26 14 41 12-15-14-24-32-29-52 11 9 25 13 40 12-18-17-29-38-38-66 19-17 30-41 30-70 0-53-33-91-82-91Z" />
      <path className="character-stage__figure" d="M140 55c-43 0-69 34-69 79 0 37 18 67 45 78-3 21-15 37-36 48 14 3 27 1 38-7-5 25-17 44-36 58 14 2 27-2 38-12-8 42-29 82-44 124-13 37-20 78-23 119h174c-3-41-10-82-23-119-15-42-36-82-44-124 11 10 24 14 38 12-19-14-31-33-36-58 11 8 24 10 38 7-21-11-33-27-36-48 27-11 45-41 45-78 0-45-26-79-69-79Z" />
      <path className="character-stage__hair" d="M84 166c-21 25-24 63-8 91m120-91c21 25 24 63 8 91M73 225c-17 30-16 64 4 87m126-87c17 30 16 64-4 87" />
    </svg>
    <div className="character-stage__status" role="status" aria-live="polite"><StatusBadge>{statusText}</StatusBadge></div>
  </>;
}
