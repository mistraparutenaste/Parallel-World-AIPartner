import type { AudioDeviceDto, RuntimeHealthEventDto, SttStateEventDto } from '@parallel-world/contracts';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MicrophonePanel } from './MicrophonePanel';

const invokeMock = vi.hoisted(() => vi.fn());
const listenHandlers = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: (name: string, handler: (event: { payload: unknown }) => void) => {
      listenHandlers.set(name, handler);
      return Promise.resolve(() => {
        listenHandlers.delete(name);
      });
    },
  }),
}));

const DEVICES: AudioDeviceDto[] = [
  { id: 'wasapi:mic-a', name: 'Mic A', is_default: false },
  { id: 'wasapi:mic-b', name: 'Mic B', is_default: true },
];

describe('MicrophonePanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_microphones') {
        return Promise.resolve(DEVICES);
      }
      if (command === 'get_stt_state') {
        return Promise.resolve({
          schema_version: 1,
          phase: 'stopped',
          message: null,
        } satisfies SttStateEventDto);
      }
      return Promise.resolve(null);
    });
  });

  it('hydrates the current state when the panel mounts after STT started', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_microphones') return Promise.resolve(DEVICES);
      if (command === 'get_stt_state') {
        return Promise.resolve({
          schema_version: 1,
          phase: 'listening',
          message: null,
        } satisfies SttStateEventDto);
      }
      return Promise.resolve(null);
    });

    render(<MicrophonePanel />);

    expect(await screen.findByText('聞き取り中')).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('get_stt_state');
  });

  it('lists devices and preselects the default one', async () => {
    render(<MicrophonePanel />);
    expect(
      await screen.findByRole('option', { name: 'Mic B（既定）' }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText('入力デバイス')).toHaveValue('wasapi:mic-b');
  });

  it('starts listening with the selected device', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });

    fireEvent.click(screen.getByRole('button', { name: '聞き取り開始' }));

    expect(invokeMock).toHaveBeenCalledWith('start_listening', {
      deviceId: 'wasapi:mic-b',
    });
  });

  it('switches the active capture device without stopping listening', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_microphones') return Promise.resolve(DEVICES);
      if (command === 'get_stt_state') {
        return Promise.resolve({
          schema_version: 1,
          phase: 'listening',
          message: null,
        } satisfies SttStateEventDto);
      }
      return Promise.resolve(null);
    });

    render(<MicrophonePanel />);
    await screen.findByText('聞き取り中');

    fireEvent.change(screen.getByRole('combobox'), {
      target: { value: 'wasapi:mic-a' },
    });

    expect(invokeMock).toHaveBeenCalledWith('set_input_device', {
      deviceId: 'wasapi:mic-a',
    });
    expect(invokeMock).not.toHaveBeenCalledWith('stop_listening');
    expect(invokeMock).not.toHaveBeenCalledWith('start_listening', expect.anything());
  });

  it('reflects unavailable state from stt-state events', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });

    const payload: SttStateEventDto = {
      schema_version: 1,
      phase: 'unavailable',
      message: 'model missing',
    };
    act(() => {
      listenHandlers.get('stt-state')?.({ payload });
    });

    expect(screen.getByRole('alert')).toHaveTextContent('model missing');
    expect(screen.getByText('利用できません')).toBeInTheDocument();
  });

  it('keeps the startup timeout message when the health circuit opens', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });

    act(() => {
      listenHandlers.get('stt-state')?.({
        payload: {
          schema_version: 1,
          phase: 'unavailable',
          message: '音声認識の起動がタイムアウトしました。停止後に再試行してください。',
        } satisfies SttStateEventDto,
      });
      listenHandlers.get('runtime-health')?.({
        payload: {
          schema_version: 1,
          feature: 'audio_input',
          status: 'recovering',
          failure_class: 'transient',
          last_error: 'operation timed out',
          attempts: 1,
          ownership: 'not_applicable',
          circuit_open: true,
          changed_at_ms: 1,
        } satisfies RuntimeHealthEventDto,
      });
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      '音声認識の起動がタイムアウトしました',
    );
  });

  it('toggles mute through set_capture_enabled', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });

    fireEvent.click(screen.getByLabelText('ミュート'));

    expect(invokeMock).toHaveBeenCalledWith('set_capture_enabled', {
      enabled: false,
    });
  });

  it('shows audio recovery and re-enumerates devices', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });
    const payload: RuntimeHealthEventDto = {
      schema_version: 1,
      feature: 'audio_input',
      status: 'recovering',
      failure_class: 'transient',
      last_error: null,
      attempts: 1,
      ownership: 'not_applicable',
      circuit_open: false,
      changed_at_ms: 1,
    };

    act(() => listenHandlers.get('runtime-health')?.({ payload }));

    expect(await screen.findByRole('alert')).toHaveTextContent('マイクを再接続しています');
    expect(invokeMock.mock.calls.filter(([name]) => name === 'list_microphones')).toHaveLength(2);
  });

  it('keeps the preferred device and fallback warning across refreshes', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });
    fireEvent.change(screen.getByRole('combobox'), {
      target: { value: 'wasapi:mic-a' },
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_microphones') return Promise.resolve([DEVICES[1]]);
      return Promise.resolve(null);
    });

    act(() => listenHandlers.get('stt-device-fallback')?.({
      payload: {
        schema_version: 1,
        preferred_device_id: 'wasapi:mic-a',
        active_device_id: 'wasapi:mic-b',
      },
    }));
    await act(async () => { await Promise.resolve(); });
    act(() => listenHandlers.get('runtime-health')?.({
      payload: {
        schema_version: 1, feature: 'audio_input', status: 'healthy',
        failure_class: null, last_error: null, attempts: 0,
        ownership: 'not_applicable', circuit_open: false, changed_at_ms: 2,
      },
    }));

    expect(screen.getByRole('combobox')).toHaveValue('wasapi:mic-a');
    expect(screen.getByRole('alert')).toBeInTheDocument();
  });
});
