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
    expect(screen.queryByRole('button', { name: '送信' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '停止' })).not.toBeInTheDocument();
  });

  it('renders control center navigation', () => {
    render(<SettingsWindow />);
    const nav = screen.getByRole('tablist', { name: '画面メニュー' });
    expect(within(nav).getByRole('tab', { name: '設定' })).toBeInTheDocument();
    expect(within(nav).getByRole('tab', { name: '性格' })).toBeInTheDocument();
    expect(within(nav).getByRole('tab', { name: '記憶' })).toBeInTheDocument();
    expect(within(nav).getByRole('tab', { name: 'チャット' })).toBeInTheDocument();
  });
});
