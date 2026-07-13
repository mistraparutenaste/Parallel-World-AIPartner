import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, vi } from 'vitest';
import { DiagnosticsPanel } from './DiagnosticsPanel';

const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }));

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  invokeMock.mockReset();
});

test('lists redacted reports and exports only to an explicit path', async () => {
  invokeMock.mockImplementation((command: string) => command === 'list_diagnostic_reports'
    ? Promise.resolve([{ id: 'crash-1.json', timestamp_ms: 42, category: 'panic', bytes: 123 }])
    : Promise.resolve(undefined));
  render(<DiagnosticsPanel />);
  expect(await screen.findByText(/crash-1.json/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('診断エクスポート先'), { target: { value: 'C:\\Temp\\diagnostics' } });
  fireEvent.click(screen.getByRole('button', { name: '診断をエクスポート' }));
  await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('export_diagnostic_reports', {
    destination: 'C:\\Temp\\diagnostics', allowOverwrite: false,
  }));
});

test('shows an overwrite failure instead of creating an unhandled rejection', async () => {
  const attempts: ReturnType<typeof deferred<unknown>>[] = [];
  invokeMock.mockImplementation((command: string) => {
    if (command === 'list_diagnostic_reports') return Promise.resolve([]);
    const attempt = deferred<unknown>();
    attempts.push(attempt);
    return attempt.promise;
  });
  const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
  render(<DiagnosticsPanel />);
  fireEvent.change(screen.getByLabelText('診断エクスポート先'), { target: { value: 'C:\\Temp\\diagnostics.json' } });
  fireEvent.click(screen.getByRole('button', { name: '診断をエクスポート' }));

  await waitFor(() => expect(attempts).toHaveLength(1));
  act(() => attempts[0]!.reject(new Error('DESTINATION_EXISTS')));
  await waitFor(() => expect(attempts).toHaveLength(2));
  act(() => attempts[1]!.reject(new Error('disk full')));

  expect(await screen.findByRole('status')).toHaveTextContent('disk full');
  expect(confirm).toHaveBeenCalledOnce();
});
