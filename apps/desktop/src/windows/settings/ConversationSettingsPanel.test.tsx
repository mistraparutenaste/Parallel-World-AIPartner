import type { BehaviorSettingsDto } from '@parallel-world/contracts';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConversationSettingsPanel } from './ConversationSettingsPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const makeSettings = (): BehaviorSettingsDto => ({
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
    if (command === 'get_behavior_settings') {
      return Promise.resolve(makeSettings());
    }
    if (command === 'set_behavior_settings') {
      return Promise.resolve(args?.settings);
    }
    return Promise.resolve(null);
  });
});

describe('conversation settings', () => {
  it('loads fail-closed defaults and saves the proactive master switch immediately', async () => {
    render(<ConversationSettingsPanel />);

    const master = await screen.findByRole('switch', {
      name: '向こうから話しかけてもらう',
    });
    expect(master).not.toBeChecked();
    expect(
      screen.getByText('現在は向こうから話しかけません。設定はONにしたときから使われます。'),
    ).toBeInTheDocument();

    fireEvent.click(master);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_behavior_settings',
        expect.objectContaining({
          settings: expect.objectContaining({ proactive_master_enabled: true }),
        }),
      ),
    );
    expect(master).toBeChecked();
  });

  it('maps the frequency slider to fixed presets and preserves trigger values while off', async () => {
    render(<ConversationSettingsPanel />);

    const frequency = await screen.findByRole('slider', { name: '話しかける頻度' });
    expect(frequency).toHaveValue('2');
    expect(screen.getByText('30分以上あけて、1日最大8回まで')).toBeInTheDocument();

    fireEvent.change(frequency, { target: { value: '3' } });
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_behavior_settings',
        expect.objectContaining({
          settings: expect.objectContaining({
            frequency: {
              minimum_interval_minutes: 15,
              max_per_hour: 3,
              max_per_day: 16,
            },
          }),
        }),
      ),
    );
    expect(screen.getByText('15分以上あけて、1日最大16回まで')).toBeInTheDocument();

    const returnSwitch = screen.getByRole('switch', { name: '戻ってきたとき' });
    fireEvent.click(returnSwitch);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenLastCalledWith(
        'set_behavior_settings',
        expect.objectContaining({
          settings: expect.objectContaining({
            triggers: expect.objectContaining({
              return_after_enabled: false,
              return_after_minutes: 10,
            }),
          }),
        }),
      ),
    );
    expect(screen.queryByRole('combobox', { name: '離れてから' })).not.toBeInTheDocument();
  });

  it('adds a valid quiet-hours row and keeps its values when disabled', async () => {
    render(<ConversationSettingsPanel />);
    await screen.findByRole('heading', { name: '話しかけない時間帯' });

    fireEvent.click(screen.getByRole('button', { name: '時間帯を追加' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        'set_behavior_settings',
        expect.objectContaining({
          settings: expect.objectContaining({
            quiet_hours: [
              expect.objectContaining({
                enabled: true,
                days_of_week: [0, 1, 2, 3, 4],
                start_local_time: '23:00',
                end_local_time: '07:00',
              }),
            ],
          }),
        }),
      ),
    );

    const row = screen.getByRole('group', { name: '話しかけない時間帯 1' });
    fireEvent.click(within(row).getByRole('switch', { name: 'この時間帯を使う' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenLastCalledWith(
        'set_behavior_settings',
        expect.objectContaining({
          settings: expect.objectContaining({
            quiet_hours: [
              expect.objectContaining({
                enabled: false,
                days_of_week: [0, 1, 2, 3, 4],
                start_local_time: '23:00',
                end_local_time: '07:00',
              }),
            ],
          }),
        }),
      ),
    );
    expect(within(row).getByLabelText('開始時刻')).toHaveValue('23:00');
    expect(within(row).getByLabelText('終了時刻')).toHaveValue('07:00');
  });

  it('sets a bounded snooze and can resume without changing the master switch', async () => {
    const now = new Date('2026-07-17T12:00:00+09:00').getTime();
    const nowSpy = vi.spyOn(Date, 'now').mockReturnValue(now);
    try {
      render(<ConversationSettingsPanel />);
      await screen.findByRole('heading', { name: 'しばらく静かにしてもらう' });

      fireEvent.click(screen.getByRole('button', { name: '時間を選ぶ' }));
      fireEvent.click(screen.getByRole('button', { name: '1時間' }));

      const expected = Math.floor(new Date('2026-07-17T13:00:00+09:00').getTime() / 1000);
      await waitFor(() =>
        expect(invokeMock).toHaveBeenCalledWith(
          'set_behavior_settings',
          expect.objectContaining({
            settings: expect.objectContaining({
              proactive_master_enabled: false,
              proactive_snoozed_until: expected,
            }),
          }),
        ),
      );
      expect(screen.getByText('13:00まで話しかけません。')).toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'また話しかけてもらう' }));
      await waitFor(() =>
        expect(invokeMock).toHaveBeenLastCalledWith(
          'set_behavior_settings',
          expect.objectContaining({
            settings: expect.objectContaining({
              proactive_master_enabled: false,
              proactive_snoozed_until: null,
            }),
          }),
        ),
      );
    } finally {
      nowSpy.mockRestore();
    }
  });

  it('requires in-place consent before enabling activity collection and stays off on failure', async () => {
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'get_behavior_settings') return Promise.resolve(makeSettings());
      if (command === 'set_behavior_settings') return Promise.reject(new Error('disk unavailable'));
      return Promise.resolve(args);
    });
    render(<ConversationSettingsPanel />);

    const collection = await screen.findByRole('switch', {
      name: '作業中の状況を参考にしてもらう',
    });
    fireEvent.click(collection);
    expect(collection).not.toBeChecked();
    expect(screen.getByText('参考にするもの')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '同意して有効にする' }));

    expect(
      await screen.findByText('今は状況を参考にできません。データ設定を確認してください。'),
    ).toBeInTheDocument();
    expect(collection).not.toBeChecked();
  });
});
