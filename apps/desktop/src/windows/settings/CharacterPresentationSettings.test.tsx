import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { CharacterPresentationSettingsDto } from '@parallel-world/contracts';
import { SettingsWindow } from './SettingsWindow';
import { tauriCharacterPresentationTransport } from '../../features/character/tauriCharacterPresentation';

vi.mock('@tauri-apps/api/core', async importOriginal => ({ ...(await importOriginal<typeof import('@tauri-apps/api/core')>()), isTauri: () => true }));

const initial: CharacterPresentationSettingsDto = { schema_version: 1, revision: 0, model_id: 'mark', expression_id: '', motion_group: 'Idle', motion_index: 0, click_through: false };

describe('character presentation settings', () => {
  it('loads typed state and writes a valid Epsilon selection', async () => {
    const save = vi.fn(async (value: CharacterPresentationSettingsDto) => value);
    render(<SettingsWindow loadCharacterPresentation={async () => initial} saveCharacterPresentation={save} />);
    fireEvent.click(screen.getByRole('button', { name: 'キャラクター' }));
    await waitFor(() => expect(screen.getByLabelText('モデル')).toHaveValue('mark'));
    fireEvent.change(screen.getByLabelText('モデル'), { target: { value: 'epsilon-free' } });
    fireEvent.change(screen.getByLabelText('表情'), { target: { value: 'Smile' } });
    fireEvent.change(screen.getByLabelText('モーション'), { target: { value: 'Tap:2' } });
    fireEvent.click(screen.getByLabelText('クリック透過'));
    fireEvent.click(screen.getByRole('button', { name: '適用' }));
    await waitFor(() => expect(save).toHaveBeenCalledWith({ schema_version: 1, revision: 0, model_id: 'epsilon-free', expression_id: 'Smile', motion_group: 'Tap', motion_index: 2, click_through: true }));
  });

  it('default loader runs once and rerenders do not overwrite a draft', async () => {
    const get = vi.spyOn(tauriCharacterPresentationTransport, 'get').mockResolvedValue(initial);
    render(<SettingsWindow />);
    fireEvent.click(screen.getByRole('button', { name: 'キャラクター' }));
    await waitFor(() => expect(get).toHaveBeenCalledOnce());
    fireEvent.change(screen.getByLabelText('モデル'), { target: { value: 'epsilon-free' } });
    fireEvent.change(screen.getByLabelText('表情'), { target: { value: 'Smile' } });
    expect(screen.getByLabelText('表情')).toHaveValue('Smile');
    expect(get).toHaveBeenCalledOnce();
  });
});
