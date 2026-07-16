import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsWindow } from './SettingsWindow';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: vi.fn().mockResolvedValue(() => {}),
  }),
}));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string) => {
    if (command === 'get_ui_preferences') {
      return Promise.resolve({
        schema_version: 1,
        theme: 'system',
        chat_placement: 'docked',
      });
    }
    if (command === 'set_theme_preference') {
      return Promise.resolve({
        schema_version: 1,
        theme: 'dark',
        chat_placement: 'docked',
      });
    }
    if (command === 'list_conversation_history') return Promise.resolve([]);
    if (command === 'list_conversation_log') {
      return Promise.resolve({
        schema_version: 1,
        messages: [],
        next_before_message_id: null,
      });
    }
    if (command === 'read_technical_log') {
      return Promise.resolve({
        schema_version: 1,
        lines: ['INFO control center ready'],
        next_cursor: { generation: 0, offset: 25 },
        reset: false,
        has_more: false,
      });
    }
    return Promise.resolve(null);
  });
});

describe('control center', () => {
  it('switches the accessible sidebar with arrow keys and pops chat out', async () => {
    render(<SettingsWindow />);
    const controlCenter = await screen.findByRole('main', { name: '管理画面' });
    expect(controlCenter).toHaveAttribute('data-ui-style', 'geometric-game');
    const navigation = screen.getByRole('tablist', { name: '管理メニュー' });
    const activePanel = screen.getByRole('tabpanel');
    expect(activePanel).toHaveAttribute(
      'aria-labelledby',
      within(navigation).getByRole('tab', { name: '会話' }).id,
    );
    const conversation = within(navigation).getByRole('tab', { name: '会話' });
    conversation.focus();
    fireEvent.keyDown(conversation, { key: 'ArrowDown' });
    expect(within(navigation).getByRole('tab', { name: '設定' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    fireEvent.click(within(navigation).getByRole('tab', { name: '会話' }));
    fireEvent.click(await screen.findByRole('button', { name: 'ポップアウト' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_chat_placement', {
        placement: 'popped',
      }),
    );
  });

  it('keeps conversation and technical logs in separate tabs', async () => {
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '管理メニュー' });
    fireEvent.click(within(navigation).getByRole('tab', { name: 'ログ' }));
    const logs = await screen.findByRole('tablist', { name: 'ログ種別' });
    expect(within(logs).getByRole('tab', { name: '会話ログ' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.queryByText('INFO control center ready')).not.toBeInTheDocument();
    fireEvent.click(within(logs).getByRole('tab', { name: '技術ログ' }));
    expect(await screen.findByText('INFO control center ready')).toBeInTheDocument();
  });

  it('applies a persisted theme choice', async () => {
    render(<SettingsWindow />);
    const theme = await screen.findByLabelText('テーマ');
    fireEvent.change(theme, { target: { value: 'dark' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_theme_preference', {
        theme: 'dark',
      }),
    );
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    act(() => {
      document.documentElement.removeAttribute('data-theme');
    });
  });

  it('renders conversation timestamps stored as Unix seconds', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') {
        return Promise.resolve({ schema_version: 1, theme: 'system', chat_placement: 'docked' });
      }
      if (command === 'list_conversation_history') return Promise.resolve([]);
      if (command === 'list_conversation_log') {
        return Promise.resolve({
          schema_version: 1,
          messages: [{ schema_version: 1, message_id: 1, turn_id: 1, role: 'user', text: 'saved', created_at: 1_700_000_000 }],
          next_before_message_id: null,
        });
      }
      return Promise.resolve(null);
    });
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '管理メニュー' });
    fireEvent.click(within(navigation).getByRole('tab', { name: 'ログ' }));
    expect(await screen.findByText(/2023/)).toBeInTheDocument();
  });

  it('surfaces a failed redock without changing the visible placement', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') {
        return Promise.resolve({ schema_version: 1, theme: 'system', chat_placement: 'popped' });
      }
      if (command === 'set_chat_placement') return Promise.reject(new Error('window unavailable'));
      return Promise.resolve([]);
    });
    render(<SettingsWindow />);
    fireEvent.click(await screen.findByRole('button', { name: '再格納' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('window unavailable');
    expect(screen.getByRole('button', { name: '再格納' })).toBeInTheDocument();
  });
});
