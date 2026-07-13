import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CharacterPresentationSettingsDto } from '@parallel-world/contracts';
import { CHARACTER_PRESENTATION_CHANGED_EVENT, type CharacterPresentationTransport } from './characterPresentation';

export const tauriCharacterPresentationTransport: CharacterPresentationTransport = {
  get: () => invoke<CharacterPresentationSettingsDto>('get_character_presentation'),
  listen: async listener => {
    const unlisten = await listen<CharacterPresentationSettingsDto>(CHARACTER_PRESENTATION_CHANGED_EVENT, event => listener(event.payload));
    return unlisten;
  },
};

export function setCharacterPresentation(value: CharacterPresentationSettingsDto) {
  return invoke<CharacterPresentationSettingsDto>('set_character_presentation', { value });
}
