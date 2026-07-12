import type { AudioDeviceDto, SttStateEventDto } from '@parallel-world/contracts';
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
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    listenHandlers.set(name, handler);
    return Promise.resolve(() => {
      listenHandlers.delete(name);
    });
  }),
}));

const DEVICES: AudioDeviceDto[] = [
  { id: 'wasapi:mic-a', name: 'Mic A', is_default: false },
  { id: 'wasapi:mic-b', name: 'Mic B', is_default: true },
];

describe('MicrophonePanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenHandlers.clear();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_microphones') {
        return Promise.resolve(DEVICES);
      }
      return Promise.resolve(null);
    });
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

  it('toggles mute through set_capture_enabled', async () => {
    render(<MicrophonePanel />);
    await screen.findByRole('option', { name: 'Mic A' });

    fireEvent.click(screen.getByLabelText('ミュート'));

    expect(invokeMock).toHaveBeenCalledWith('set_capture_enabled', {
      enabled: false,
    });
  });
});
