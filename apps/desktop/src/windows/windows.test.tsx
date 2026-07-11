import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { CharacterWindow } from './character/CharacterWindow';
import { ChatWindow } from './chat/ChatWindow';
import { SettingsWindow } from './settings/SettingsWindow';

describe('desktop windows', () => {
  it('renders the character surface', () => {
    render(<CharacterWindow />);
    expect(screen.getByRole('status')).toHaveTextContent('準備中');
  });

  it('renders chat input and stop action', () => {
    render(<ChatWindow />);
    expect(screen.getByLabelText('メッセージ')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument();
  });

  it('renders settings navigation', () => {
    render(<SettingsWindow />);
    expect(screen.getByRole('heading', { name: '設定' })).toBeInTheDocument();
    expect(screen.getByText('マイク')).toBeInTheDocument();
  });
});
