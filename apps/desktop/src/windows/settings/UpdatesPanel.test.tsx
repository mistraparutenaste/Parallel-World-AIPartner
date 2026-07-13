import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { UpdatesPanel } from './UpdatesPanel';

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

const available = {
  schema_version: 1,
  status: 'available',
  current_version: '1.0.0',
  available_version: '1.1.0',
  notes: '重要な更新です',
  error: null,
};

test('shows update metadata and only installs the explicitly displayed version', async () => {
  invokeMock.mockImplementation((command) => command === 'get_update_state'
    ? Promise.resolve(available)
    : Promise.resolve(undefined));
  render(<UpdatesPanel />);

  expect(await screen.findByText('利用可能なバージョン: 1.1.0')).toBeInTheDocument();
  expect(screen.getByText('重要な更新です')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /1\.1\.0 をインストール/ }));
  await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('install_update', {
    approvedVersion: '1.1.0',
  }));
});

test('manual check refreshes state and disables actions while active', async () => {
  invokeMock.mockImplementation((command) => {
    if (command === 'get_update_state') return Promise.resolve({ ...available, status: 'checking' });
    if (command === 'check_for_updates') return Promise.resolve(available);
    return Promise.resolve(undefined);
  });
  render(<UpdatesPanel />);
  const button = await screen.findByRole('button', { name: '更新を確認' });
  expect(button).toBeDisabled();
});
