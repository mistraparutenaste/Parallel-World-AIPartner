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

const makeBehaviorSettings = () => ({
  schema_version: 2,
  proactive_master_enabled: false,
  consent: 'pending',
  consent_version: 1,
  collection_enabled: false,
  retention_days: 30,
  exclusions: [],
  frequency: {
    minimum_interval_minutes: 30,
    max_per_hour: 2,
    max_per_day: 8,
  },
  triggers: {
    return_after_enabled: true,
    return_after_minutes: 10,
    long_session_enabled: true,
    long_session_minutes: 60,
    category_change_enabled: true,
    category_change_minutes: 10,
  },
  quiet_hours: [],
  proactive_snoozed_until: null,
  evaluator_endpoint: null,
  evaluator_model: null,
  shortcuts: {
    push_to_talk: 'Ctrl+Alt+Space',
    toggle_mute: 'Ctrl+Alt+M',
    open_control_center: 'Ctrl+Alt+P',
    toggle_character: 'Ctrl+Alt+C',
    cycle_mode: 'Ctrl+Alt+F',
  },
  profiles: {
    normal: {
      proactive_enabled: true,
      tts_enabled: true,
      character_enabled: true,
      notifications_enabled: false,
      volume: 1,
    },
    focus: {
      proactive_enabled: false,
      tts_enabled: false,
      character_enabled: false,
      notifications_enabled: false,
      volume: 0,
    },
    night: {
      proactive_enabled: false,
      tts_enabled: false,
      character_enabled: false,
      notifications_enabled: false,
      volume: 0,
    },
  },
  activation: {
    schedules: [],
    apps: [],
    fullscreen: {
      enabled: false,
      mode: 'focus',
    },
  },
  manual_mode_override: null,
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
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
        theme: args?.theme ?? 'system',
        chat_placement: 'docked',
      });
    }
    if (command === 'set_chat_placement') {
      return Promise.resolve({
        schema_version: 1,
        theme: 'system',
        chat_placement: args?.placement ?? 'docked',
      });
    }
    if (command === 'get_behavior_settings') return Promise.resolve(makeBehaviorSettings());
    if (command === 'set_behavior_settings') return Promise.resolve(args?.settings);
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
    if (command === 'list_diagnostic_reports') return Promise.resolve([]);
    if (command === 'get_runtime_diagnostics') {
      return Promise.resolve({
        schema_version: 1,
        queues: [],
        health: [],
      });
    }
    return Promise.resolve(null);
  });
});

describe('control center', () => {
  it('renders a conversation-first shell without the old sidebar', async () => {
    render(<SettingsWindow />);
    const controlCenter = await screen.findByRole('main', { name: 'Parallel World' });
    expect(controlCenter).toHaveAttribute('data-ui-style', 'conversation-first');
    expect(controlCenter.querySelector('.control-sidebar')).not.toBeInTheDocument();

    const navigation = screen.getByRole('tablist', { name: '画面メニュー' });
    expect(within(navigation).getByRole('tab', { name: 'チャット' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    const chat = within(navigation).getByRole('tab', { name: 'チャット' });
    chat.focus();
    fireEvent.keyDown(chat, { key: 'ArrowRight' });
    expect(screen.queryByRole('tablist', { name: '画面メニュー' })).not.toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: '設定画面' })).toBeInTheDocument();
  });

  it('uses the same close-only screen for settings, personality, and conversation', async () => {
    render(<SettingsWindow />);

    for (const label of ['設定', '性格', '会話']) {
      const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
      fireEvent.click(within(navigation).getByRole('tab', { name: label }));

      const focusedScreen = await screen.findByRole('dialog', { name: `${label}画面` });
      expect(screen.queryByRole('tablist', { name: '画面メニュー' })).not.toBeInTheDocument();

      const close = within(focusedScreen).getByRole('button', { name: `${label}を閉じる` });
      await waitFor(() => expect(close).toHaveFocus());
      fireEvent.click(close);

      const restoredNavigation = await screen.findByRole('tablist', {
        name: '画面メニュー',
      });
      await waitFor(() =>
        expect(within(restoredNavigation).getByRole('tab', { name: 'チャット' })).toHaveFocus(),
      );
      expect(screen.queryByRole('dialog', { name: `${label}画面` })).not.toBeInTheDocument();
    }
  });

  it('keeps the existing conversation layer beneath the settings entrance', async () => {
    render(<SettingsWindow />);
    const controlCenter = await screen.findByRole('main', { name: 'Parallel World' });
    const conversationLayer = controlCenter.querySelector('.conversation-layer');
    const navigation = screen.getByRole('tablist', { name: '画面メニュー' });

    fireEvent.click(within(navigation).getByRole('tab', { name: '設定' }));

    expect(controlCenter).toHaveAttribute('data-transition', 'settings');
    expect(conversationLayer).toBeInTheDocument();
    expect(conversationLayer).toHaveAttribute('aria-hidden', 'true');
    const transitionNavigation = controlCenter.querySelector('.screen-navigation');
    expect(transitionNavigation).toBeInTheDocument();
    expect(transitionNavigation?.querySelector('[data-tab-id="settings"]')).toHaveAttribute(
      'data-confirming',
      'true',
    );

    const categories = screen.getByRole('tablist', { name: '設定カテゴリ' });
    const entranceOrder = ['音声', '診断', 'AI', '表示', 'キャラクター', '更新', 'データ'];
    entranceOrder.forEach((label, index) => {
      expect(within(categories).getByRole('tab', { name: label })).toHaveAttribute(
        'data-enter-order',
        String(index),
      );
    });
  });

  it.each([
    ['性格', 'personality', 'diamond'],
    ['会話', 'conversation', 'circle'],
  ] as const)('uses three %s transition outlines from the clicked diamond', async (label, target, shape) => {
    render(<SettingsWindow />);
    const controlCenter = await screen.findByRole('main', { name: 'Parallel World' });
    const navigation = screen.getByRole('tablist', { name: '画面メニュー' });

    fireEvent.click(within(navigation).getByRole('tab', { name: label }));

    expect(controlCenter).toHaveAttribute('data-transition', target);
    expect(screen.getByRole('dialog', { name: `${label}画面` })).toHaveAttribute(
      'data-motion-shape',
      shape,
    );
    expect(controlCenter.querySelectorAll(`[data-transition-ring="${shape}"]`)).toHaveLength(3);
    expect(
      controlCenter.querySelector(`.screen-navigation [data-tab-id="${target}"]`),
    ).toHaveAttribute('data-confirming', 'true');
  });

  it('uses the seven-category heart crown and keeps logs under data and diagnostics', async () => {
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
    fireEvent.click(within(navigation).getByRole('tab', { name: '設定' }));

    const categories = screen.getByRole('tablist', { name: '設定カテゴリ' });
    expect(within(categories).getAllByRole('tab')).toHaveLength(7);
    for (const label of ['音声', 'AI', 'キャラクター', 'データ', '診断', '表示', '更新']) {
      expect(within(categories).getByRole('tab', { name: label })).toBeInTheDocument();
    }
    expect(within(categories).getByRole('tab', { name: '表示' })).toHaveAttribute(
      'aria-selected',
      'true',
    );

    fireEvent.click(within(categories).getByRole('tab', { name: 'データ' }));
    expect(await screen.findByRole('region', { name: '会話ログ' })).toBeInTheDocument();

    fireEvent.click(within(categories).getByRole('tab', { name: '診断' }));
    expect(await screen.findByRole('region', { name: '技術ログ' })).toBeInTheDocument();
    expect(await screen.findByText('INFO control center ready')).toBeInTheDocument();
  });

  it('keeps theme and chat placement inside display settings', async () => {
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
    expect(screen.queryByRole('combobox', { name: 'テーマ' })).not.toBeInTheDocument();
    fireEvent.click(within(navigation).getByRole('tab', { name: '設定' }));

    const theme = await screen.findByRole('combobox', { name: 'テーマ' });
    fireEvent.change(theme, { target: { value: 'dark' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_theme_preference', {
        theme: 'dark',
      }),
    );
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');

    fireEvent.change(screen.getByRole('combobox', { name: 'チャット表示' }), {
      target: { value: 'popped' },
    });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_chat_placement', {
        placement: 'popped',
      }),
    );
    act(() => {
      document.documentElement.removeAttribute('data-theme');
    });
  });

  it('opens the complete user-wide conversation settings from the heart menu', async () => {
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });

    fireEvent.click(within(navigation).getByRole('tab', { name: '会話' }));

    expect(await screen.findByRole('region', { name: '会話設定' })).toBeInTheDocument();
    expect(
      screen.getByRole('switch', { name: '向こうから話しかけてもらう' }),
    ).not.toBeChecked();
    expect(screen.getByRole('heading', { name: '話しかけない時間帯' })).toBeInTheDocument();
    expect(
      screen.getByRole('switch', { name: '作業中の状況を参考にしてもらう' }),
    ).not.toBeChecked();
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
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
    fireEvent.click(within(navigation).getByRole('tab', { name: '設定' }));
    const categories = screen.getByRole('tablist', { name: '設定カテゴリ' });
    fireEvent.click(within(categories).getByRole('tab', { name: 'データ' }));
    expect(await screen.findByText(/2023/)).toBeInTheDocument();
  });

  it('refocuses an already popped chat without showing a replacement panel', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') {
        return Promise.resolve({ schema_version: 1, theme: 'system', chat_placement: 'popped' });
      }
      if (command === 'set_chat_placement') {
        return Promise.resolve({ schema_version: 1, theme: 'system', chat_placement: 'popped' });
      }
      if (command === 'list_conversation_history') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
    expect(screen.queryByText(/別ウィンドウで開いています/)).not.toBeInTheDocument();
    fireEvent.click(within(navigation).getByRole('tab', { name: 'チャット' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_chat_placement', {
        placement: 'popped',
      }),
    );
  });

  it('surfaces a failed placement change without changing the selected value', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') {
        return Promise.resolve({ schema_version: 1, theme: 'system', chat_placement: 'popped' });
      }
      if (command === 'set_chat_placement') return Promise.reject(new Error('window unavailable'));
      if (command === 'list_conversation_history') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<SettingsWindow />);
    const navigation = await screen.findByRole('tablist', { name: '画面メニュー' });
    fireEvent.click(within(navigation).getByRole('tab', { name: '設定' }));
    const placement = await screen.findByRole('combobox', { name: 'チャット表示' });
    fireEvent.change(placement, { target: { value: 'docked' } });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'チャット表示を変更できません。少し待って、もう一度試してください。',
    );
    expect(placement).toHaveValue('popped');
  });
});
