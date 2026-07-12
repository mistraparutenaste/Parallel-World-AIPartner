import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { CharacterWindow } from './character/CharacterWindow';
import { ChatWindow } from './chat/ChatWindow';
import { SettingsWindow } from './settings/SettingsWindow';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockRejectedValue(new Error('tauri is not available')),
  convertFileSrc: (path: string) => path,
}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: vi.fn().mockResolvedValue(() => {}),
  }),
}));

describe('desktop windows', () => {
  it('renders the character surface and degrades without the core script', async () => {
    render(<CharacterWindow />);
    expect(screen.getByRole('status')).toBeInTheDocument();
    expect(
      await screen.findByText('キャラクター表示が利用できません'),
    ).toBeInTheDocument();
  });

  it('renders chat input and stop action', () => {
    render(<ChatWindow />);
    expect(screen.getByLabelText('メッセージ')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument();
  });

  it('renders settings navigation', () => {
    render(<SettingsWindow />);
    expect(screen.getByRole('heading', { name: '設定' })).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: '設定メニュー' });
    expect(within(nav).getByText('マイク')).toBeInTheDocument();
  });
});
