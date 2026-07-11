import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { CharacterWindow } from './character/CharacterWindow';
import { ChatWindow, initialAppStatus } from './chat/ChatWindow';
import { SettingsWindow } from './settings/SettingsWindow';

describe('desktop windows', () => {
  it('renders the character surface', () => {
    const { container } = render(<CharacterWindow />);
    expect(screen.getByRole('status')).toHaveTextContent('準備中');
    expect(container.querySelector('.character-stage__silhouette')).toHaveAttribute('aria-hidden', 'true');
    expect(container.querySelector('.character-stage__drag')).toHaveAttribute('data-tauri-drag-region');
  });

  it('keeps the complete character document transparent', () => {
    const desktopRoot = process.cwd();
    const html = readFileSync(resolve(desktopRoot, 'character.html'), 'utf8');
    const css = readFileSync(resolve(desktopRoot, 'src/shared/styles/global.css'), 'utf8');
    expect(html).toContain('<html class="character-document"');
    expect(css).toContain('html.character-document, .character-body, .character-body #root { background: transparent; }');
  });

  it('renders chat input and stop action', () => {
    render(<ChatWindow />);
    expect(screen.getByLabelText('メッセージ')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '停止' })).toBeInTheDocument();
  });

  it('uses the versioned app status contract for the initial chat state', () => {
    expect(initialAppStatus).toEqual({ schema_version: 1, conversation_state: 'idle' });
  });

  it('renders settings navigation', () => {
    render(<SettingsWindow />);
    expect(screen.getByRole('heading', { name: '設定' })).toBeInTheDocument();
    expect(screen.getByText('マイク')).toBeInTheDocument();
  });

  it('submits a local message and lets the user stop processing', () => {
    render(<ChatWindow />);
    const input = screen.getByLabelText('メッセージ');
    fireEvent.change(input, { target: { value: 'こんにちは' } });
    fireEvent.click(screen.getByRole('button', { name: '送信' }));
    expect(screen.getByText('こんにちは')).toBeInTheDocument();
    expect(input).toHaveValue('');
    const stop = screen.getByRole('button', { name: '停止' });
    expect(stop).toBeEnabled();
    fireEvent.click(stop);
    expect(stop).toBeDisabled();
  });

  it('updates the selected settings section locally', () => {
    render(<SettingsWindow />);
    const llm = screen.getByRole('button', { name: 'LLM' });
    fireEvent.click(llm);
    expect(llm).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('heading', { name: '設定' })).toHaveTextContent(/^設定$/);
    expect(screen.queryByLabelText('マイクデバイス')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'マイク' }));
    expect(screen.getByLabelText('マイクデバイス')).toBeInTheDocument();
  });
});
