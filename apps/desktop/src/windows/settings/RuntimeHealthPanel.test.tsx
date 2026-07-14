import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { vi } from 'vitest';
import { mergeHealthEvents, RuntimeHealthPanel } from './RuntimeHealthPanel';

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
    circuit_open: true,
    changed_at_ms: 43,
  });
  expect(await screen.findByText(/circuit open/)).toBeInTheDocument();
  expect(screen.getByText(/external/)).toBeInTheDocument();
});

test('settings rearms a circuit-open runtime feature', async () => {
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
  expect(invokeMock).toHaveBeenCalledWith('rearm_runtime_feature', {
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
  expect(screen.getByRole('button', { name: /音声合成.*再起動/ })).toBeInTheDocument();
});

test('settings exposes character renderer retry after the first failed boot', async () => {
  render(<RuntimeHealthPanel />);
  await publish({
    schema_version: 1,
    feature: 'character_renderer',
    status: 'recovering',
    failure_class: 'transient',
    last_error: 'renderer initialization failed',
    attempts: 1,
    ownership: 'not_applicable',
    circuit_open: false,
    changed_at_ms: 46,
  });

  fireEvent.click(await screen.findByRole('button', { name: 'キャラクター表示 を再起動' }));
  expect(invokeMock).toHaveBeenCalledWith('rearm_runtime_feature', { feature: 'character_renderer' });
});

test('renders all bounded queue diagnostics', async () => {
  invokeMock.mockImplementation((command) => command === 'get_runtime_diagnostics'
    ? Promise.resolve({ schema_version: 1, queues: [
      { name: 'chat_submit', depth: 2, capacity: 8, dropped: 1, busy: 1, coalesced: 0 },
      { name: 'tts', depth: 0, capacity: 8, dropped: 3, busy: 2, coalesced: 0 },
    ] })
    : Promise.resolve(undefined));
  render(<RuntimeHealthPanel />);
  expect(await screen.findByText(/chat_submit.*2 \/ 8.*dropped 1.*busy 1/)).toBeInTheDocument();
  expect(screen.getByText(/tts.*0 \/ 8.*dropped 3.*busy 2/)).toBeInTheDocument();
});

test('refreshes queue diagnostics periodically and stops after unmount', async () => {
  vi.useFakeTimers();
  invokeMock.mockResolvedValue({ schema_version: 1, queues: [] });
  const view = render(<RuntimeHealthPanel />);
  await act(async () => { await Promise.resolve(); });
  const before = invokeMock.mock.calls.filter(([name]) => name === 'get_runtime_diagnostics').length;
  await act(async () => { vi.advanceTimersByTime(5_000); await Promise.resolve(); });
  expect(invokeMock.mock.calls.filter(([name]) => name === 'get_runtime_diagnostics')).toHaveLength(before + 1);
  view.unmount();
  vi.advanceTimersByTime(10_000);
  expect(invokeMock.mock.calls.filter(([name]) => name === 'get_runtime_diagnostics')).toHaveLength(before + 1);
  vi.useRealTimers();
});

test('loads current circuit state when Settings opens after the event', async () => {
  invokeMock.mockImplementation((command) => command === 'get_runtime_diagnostics'
    ? Promise.resolve({ schema_version: 1, queues: [], health: [{
      schema_version: 1,
      feature: 'language_model',
      status: 'degraded',
      failure_class: 'transient',
      last_error: 'endpoint unavailable',
      attempts: 8,
      ownership: 'not_applicable',
      circuit_open: true,
      changed_at_ms: 47,
    }] })
    : Promise.resolve(undefined));
  render(<RuntimeHealthPanel />);
  expect(await screen.findByRole('button', { name: 'LLM を再起動' })).toBeInTheDocument();
});

test('older service snapshot does not overwrite a newer managed circuit event', () => {
  const managed = { feature: 'language_model', ownership: 'managed', changed_at_ms: 100, circuit_open: true } as any;
  const olderManaged = { ...managed, changed_at_ms: 50, circuit_open: false } as any;
  expect(mergeHealthEvents({ 'language_model:managed': managed }, [olderManaged])['language_model:managed']).toBe(managed);
});

test('managed and application health for one feature remain separate', () => {
  const managed = { feature: 'language_model', ownership: 'managed', changed_at_ms: 100 } as any;
  const application = { feature: 'language_model', ownership: 'not_applicable', changed_at_ms: 101 } as any;
  const merged = mergeHealthEvents({}, [managed, application]);
  expect(Object.values(merged)).toEqual(expect.arrayContaining([managed, application]));
  expect(Object.values(merged)).toHaveLength(2);
});

test('loads a recovering managed process when Settings opens after its event', async () => {
  invokeMock.mockImplementation((command) => command === 'get_runtime_diagnostics'
    ? Promise.resolve({ schema_version: 1, queues: [], health: [{
      schema_version: 1,
      feature: 'language_model',
      status: 'recovering',
      failure_class: 'transient',
      last_error: 'process unavailable',
      attempts: 3,
      ownership: 'managed',
      circuit_open: false,
      changed_at_ms: 88,
    }] })
    : Promise.resolve(undefined));
  render(<RuntimeHealthPanel />);
  expect(await screen.findByText(/managed/)).toBeInTheDocument();
  expect(screen.getByText(/retry 3/)).toBeInTheDocument();
});
