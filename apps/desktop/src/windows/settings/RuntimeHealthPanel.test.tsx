import { render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { vi } from 'vitest';
import { RuntimeHealthPanel } from './RuntimeHealthPanel';

let listener: ((event: { payload: any }) => void) | undefined;
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_name, callback) => {
    listener = callback;
    return () => undefined;
  }),
}));

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
