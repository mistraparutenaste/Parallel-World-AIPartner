import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { vi } from 'vitest';
import { RuntimeHealthPanel } from './RuntimeHealthPanel';

let listener: ((event: { payload: any }) => void) | undefined;
const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name, callback) => {
    listener = callback;
    return () => undefined;
  }),
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

async function publish(payload: any) {
  await waitFor(() => expect(listener).toBeDefined());
  await act(async () => listener?.({ payload }));
}

test('runtime health event renders ownership and retry state', async () => {
  render(<RuntimeHealthPanel />);
  await publish({
    schema_version: 1,
    feature: 'language_model',
    status: 'recovering',
    failure_class: 'transient',
    last_error: 'owned process exited',
    attempts: 3,
    changed_at_ms: 42,
  });
  expect(await screen.findByText('LLM')).toBeInTheDocument();
  expect(screen.getByText(/owned/)).toBeInTheDocument();
  expect(screen.getByText(/3/)).toBeInTheDocument();
});

test('eight attempts are shown as an open circuit', async () => {
  render(<RuntimeHealthPanel />);
  await publish({
    schema_version: 1,
    feature: 'text_to_speech',
    status: 'failed',
    failure_class: 'persistent',
    last_error: 'external endpoint unavailable',
    attempts: 8,
    changed_at_ms: 43,
  });
  expect(await screen.findByText(/circuit open/)).toBeInTheDocument();
  expect(screen.getByText(/external/)).toBeInTheDocument();
});

test('settings can rearm only a circuit-open managed process', async () => {
  render(<RuntimeHealthPanel />);
  await publish({
    schema_version: 1,
    feature: 'language_model',
    status: 'degraded',
    failure_class: 'transient',
    last_error: 'managed process restart circuit opened',
    attempts: 8,
    ownership: 'managed',
    circuit_open: true,
    changed_at_ms: 44,
  });

  fireEvent.click(await screen.findByRole('button', { name: 'LLM を再起動' }));
  expect(invokeMock).toHaveBeenCalledWith('rearm_managed_process', {
    feature: 'language_model',
  });

  await publish({
    schema_version: 1,
    feature: 'text_to_speech',
    status: 'degraded',
    failure_class: 'transient',
    last_error: 'external endpoint unavailable',
    attempts: 8,
    ownership: 'external',
    circuit_open: true,
    changed_at_ms: 45,
  });
  expect(screen.queryByRole('button', { name: /音声合成.*再起動/ })).not.toBeInTheDocument();
});
