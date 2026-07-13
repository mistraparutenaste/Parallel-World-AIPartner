import { useEffect, useState, type RefObject } from 'react';
import { Live2DError, type CharacterController, type CharacterModelSource, type CharacterRuntimeStatus, type Live2DErrorCode } from '@parallel-world/live2d-runtime';
import { bindCharacterPresentation, type CharacterModelSourceResolver, type CharacterPresentationTransport } from './characterPresentation';

export type CharacterControllerFactory = () => CharacterController | Promise<CharacterController>;
export interface CharacterControllerViewState { status: CharacterRuntimeStatus; errorCode?: Live2DErrorCode }
const normalizeError = (error: unknown): Live2DErrorCode => error instanceof Live2DError ? error.code : 'mount-failed';

export function useCharacterController(canvasRef: RefObject<HTMLCanvasElement | null>, controllerFactory: CharacterControllerFactory, modelSource: CharacterModelSource, presentationTransport?: CharacterPresentationTransport, resolvePresentationModel?: CharacterModelSourceResolver): CharacterControllerViewState {
  const [viewState, setViewState] = useState<CharacterControllerViewState>({ status: { kind: 'loading', modelId: modelSource.modelId } });
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let active = true;
    let controller: CharacterController | undefined;
    let unsubscribe: (() => void) | undefined;
    let presentationBinding: ReturnType<typeof bindCharacterPresentation> | undefined;
    setViewState({ status: { kind: 'loading', modelId: modelSource.modelId } });
    void Promise.resolve().then(controllerFactory).then(async created => {
      if (!active) { created.dispose(); return; }
      controller = created;
      unsubscribe = created.subscribe(status => {
        if (!active || status.kind === 'idle') return;
        setViewState({ status, errorCode: status.kind === 'failed' ? status.code : undefined });
      });
      await created.mount(canvas);
      if (!active) return;
      created.resize({ cssWidth: canvas.clientWidth, cssHeight: canvas.clientHeight, devicePixelRatio: globalThis.devicePixelRatio || 1 });
      if (presentationTransport) {
        presentationBinding = bindCharacterPresentation(created, presentationTransport, resolvePresentationModel);
        await presentationBinding.ready;
      } else {
        await created.loadModel(modelSource);
      }
    }).catch(error => {
      if (!active) return;
      const code = normalizeError(error);
      setViewState({ status: { kind: 'failed', modelId: modelSource.modelId, code }, errorCode: code });
    });
    return () => {
      active = false;
      try { unsubscribe?.(); } catch { /* Runtime cleanup is best-effort and must continue. */ }
      try { presentationBinding?.dispose(); } catch { /* Event cleanup must not block controller cleanup. */ }
      try { controller?.dispose(); } catch { /* Isolate injected controller cleanup failures. */ }
    };
  }, [canvasRef, controllerFactory, modelSource, presentationTransport, resolvePresentationModel]);
  return viewState;
}
