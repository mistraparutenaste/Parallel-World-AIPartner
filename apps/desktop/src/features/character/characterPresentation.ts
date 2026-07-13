import type { CharacterPresentationSettingsDto } from '@parallel-world/contracts';
import type { CharacterController, CharacterModelSource } from '@parallel-world/live2d-runtime';

export const CHARACTER_PRESENTATION_SCHEMA_VERSION = 1;
export const CHARACTER_PRESENTATION_CHANGED_EVENT = 'character-presentation://changed';

export interface CharacterPresentationTransport {
  get(): Promise<unknown>;
  listen(listener: (value: unknown) => void): Promise<() => void>;
}

export function isCurrentCharacterPresentation(value: unknown): value is CharacterPresentationSettingsDto {
  if (!value || typeof value !== 'object') return false;
  const candidate = value as Record<string, unknown>;
  const shapeValid = candidate.schema_version === CHARACTER_PRESENTATION_SCHEMA_VERSION
    && Number.isSafeInteger(candidate.revision) && (candidate.revision as number) >= 0
    && typeof candidate.model_id === 'string' && typeof candidate.expression_id === 'string'
    && typeof candidate.motion_group === 'string' && Number.isInteger(candidate.motion_index)
    && typeof candidate.click_through === 'boolean';
  if (!shapeValid) return false;
  const model = candidate.model_id as string;
  const expression = candidate.expression_id as string;
  const group = candidate.motion_group as string;
  const index = candidate.motion_index as number;
  const expressionValid = model === 'mark' ? expression === '' : model === 'epsilon-free' && ['Angry', 'Blushing', 'f01', 'f02', 'Normal', 'Sad', 'Smile', 'Surprised'].includes(expression);
  const limit = model === 'mark' && group === 'Idle' ? 6 : model === 'epsilon-free' ? ({ Idle: 1, FlickUp: 2, Flick: 2, Tap: 4, Flick3: 2, FlickDown: 2, Shake: 2 } as Record<string, number>)[group] : undefined;
  return expressionValid && limit !== undefined && index >= 0 && index < limit;
}

export type CharacterModelSourceResolver = (modelId: string) => CharacterModelSource;
export const characterModelSource: CharacterModelSourceResolver = modelId => ({ modelId, manifestUrl: `characters/${modelId}/character.json` });

export function bindCharacterPresentation(controller: CharacterController, transport: CharacterPresentationTransport, resolveModelSource: CharacterModelSourceResolver = characterModelSource) {
  let active = true;
  let currentModelId: string | undefined;
  let scheduledRevision = -1;
  let unlisten: (() => void) | undefined;
  let pending = Promise.resolve();
  let failure: unknown;
  const removeListener = () => { const remove = unlisten; unlisten = undefined; remove?.(); };
  const enqueue = (unknownValue: unknown) => {
    if (!isCurrentCharacterPresentation(unknownValue)) return;
    if (unknownValue.revision <= scheduledRevision) return;
    scheduledRevision = unknownValue.revision;
    pending = pending.then(async () => {
      if (!active) return;
      if (unknownValue.model_id !== currentModelId) {
        await controller.loadModel(resolveModelSource(unknownValue.model_id));
        currentModelId = unknownValue.model_id;
      }
      if (unknownValue.expression_id) await controller.setExpression(unknownValue.expression_id);
      await controller.playMotion(unknownValue.motion_group, unknownValue.motion_index);
    }).catch(error => { failure = error; removeListener(); });
  };
  const ready = (async () => {
    try {
      const registeredUnlisten = await transport.listen(enqueue);
      if (!active) { registeredUnlisten(); return; }
      unlisten = registeredUnlisten;
      enqueue(await transport.get());
      await pending;
      if (failure) throw failure;
    } catch (error) {
      removeListener();
      throw error;
    }
  })();
  return {
    ready,
    flush: async () => { await pending; if (failure) throw failure; },
    dispose: () => { active = false; removeListener(); },
  };
}
