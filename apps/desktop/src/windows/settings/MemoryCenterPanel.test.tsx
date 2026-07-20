import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { vi } from 'vitest';
import { MemoryCenterPanel } from './MemoryCenterPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

describe('MemoryCenterPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_memory_center') {
        return Promise.resolve({
          schema_version: 1,
          domains: [{ domain: 'semantic_user', consent: 'allowed', retention_seconds: null, revision: 0 }],
          pending: [{ id: 4, domain: 'semantic_user', preview: 'Likes tea', created_at: 1 }],
          commitments: [],
          dialogue: null,
          temporary: false,
          temporary_revision: 0,
        });
      }
      return Promise.resolve(null);
    });
  });

  it('hydrates bounded previews and changes temporary mode without exposing a transcript', async () => {
    render(<MemoryCenterPanel />);
    expect((await screen.findByRole('listitem')).textContent).toContain('Likes tea');
    expect(screen.queryByText(/transcript/i)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('switch', { name: '一時会話モード' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_temporary_conversation', {
      temporary: true,
      expectedRevision: 0,
    }));
  });
});
