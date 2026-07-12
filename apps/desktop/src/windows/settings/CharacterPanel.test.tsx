import type { CharacterManifestDto } from '@parallel-world/contracts';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CharacterPanel } from './CharacterPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const MANIFEST: CharacterManifestDto = {
  schema_version: 1,
  model_path: 'C:/data/characters/epsilon/Epsilon.model3.json',
  expressions: ['Normal', 'Smile'],
  motion_groups: [
    { name: 'Idle', motion_count: 1 },
    { name: 'Tap', motion_count: 4 },
  ],
};

describe('CharacterPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('lists expressions and motion groups from the manifest', async () => {
    invokeMock.mockResolvedValueOnce(MANIFEST);
    render(<CharacterPanel />);

    expect(
      await screen.findByRole('option', { name: 'Smile' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Tap を再生' }),
    ).toBeInTheDocument();
  });

  it('sends the selected expression through the command', async () => {
    invokeMock.mockResolvedValue(MANIFEST);
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情');

    fireEvent.change(select, { target: { value: 'Smile' } });

    expect(invokeMock).toHaveBeenCalledWith('set_expression', {
      name: 'Smile',
    });
  });

  it('starts a motion group through the command', async () => {
    invokeMock.mockResolvedValue(MANIFEST);
    render(<CharacterPanel />);
    const button = await screen.findByRole('button', { name: 'Idle を再生' });

    fireEvent.click(button);

    expect(invokeMock).toHaveBeenCalledWith('start_motion', { group: 'Idle' });
  });

  it('shows an alert when the manifest cannot be loaded', async () => {
    invokeMock.mockRejectedValueOnce(new Error('no character model'));
    render(<CharacterPanel />);

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'キャラクターモデルを読み込めません',
    );
  });

  it('reloads the manifest when retry is clicked after a failure', async () => {
    invokeMock.mockRejectedValueOnce(new Error('no character model'));
    invokeMock.mockResolvedValueOnce(MANIFEST);
    render(<CharacterPanel />);
    const retry = await screen.findByRole('button', { name: '再読み込み' });

    fireEvent.click(retry);

    expect(
      await screen.findByRole('option', { name: 'Smile' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
