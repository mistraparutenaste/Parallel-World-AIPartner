import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { MemoryCenterPanel } from './MemoryCenterPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const snapshot = (memories = [{ id: 7, preview: 'Likes tea', state: 'active', pinned: false, created_at: 1, updated_at: 1, revision: 1 }]) => ({
  schema_version: 1,
  domains: [{ domain: 'semantic_user', consent: 'allowed', retention_seconds: null, revision: 0 }],
  memories,
  pending: [{ id: 4, domain: 'semantic_user', preview: 'Likes tea', created_at: 1 }],
  commitments: [],
  dialogue: null,
  temporary: false,
  temporary_revision: 0,
});

describe('MemoryCenterPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_memory_center') return Promise.resolve(snapshot());
      return Promise.resolve(null);
    });
  });

  it('hydrates bounded previews and changes temporary mode without exposing a transcript', async () => {
    render(<MemoryCenterPanel />);
    expect((await screen.findAllByRole('listitem'))[0]?.textContent).toContain('Likes tea');
    expect(screen.queryByText(/transcript/i)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('switch', { name: '一時会話モード' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_temporary_conversation', {
        temporary: true,
        expectedRevision: 0,
      }),
    );
  });

  it('deletes one memory after confirmation and refreshes the bounded snapshot', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_memory_center') return Promise.resolve(snapshot());
      if (command === 'delete_memory') return Promise.resolve(snapshot([]));
      return Promise.resolve(null);
    });
    render(<MemoryCenterPanel />);
    await screen.findByText('Likes tea');
    fireEvent.click(screen.getByRole('button', { name: 'メモリー 7 を削除' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('delete_memory', { memoryId: 7 }),
    );
    await waitFor(() =>
      expect(screen.getByRole('heading', { name: /保存済みメモリー/ })).toHaveTextContent('(0)'),
    );
  });
});
