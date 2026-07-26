import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SavedMemoriesPanel } from './SavedMemoriesPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const memory = { id: 1, preview: 'preview', state: 'active', pinned: false, created_at: 1, updated_at: 1, revision: 1 };
const center = { schema_version: 1, domains: [], memories: [memory], pending: [], commitments: [], dialogue: null, temporary: false, temporary_revision: 0 };

describe('SavedMemoriesPanel', () => {
  beforeEach(() => invokeMock.mockReset());

  it('loads full content only after edit and saves through CAS', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_memory_center' || command === 'update_memory') return Promise.resolve(center);
      if (command === 'get_memory_content') return Promise.resolve('full content');
      return Promise.resolve(null);
    });
    render(<SavedMemoriesPanel />);
    fireEvent.click(await screen.findByRole('button', { name: '編集' }));
    const textarea = await screen.findByRole('textbox', { name: '内容' });
    expect(textarea).toHaveValue('full content');
    fireEvent.change(textarea, { target: { value: 'updated content' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('update_memory', {
      memoryId: 1, content: 'updated content', expectedRevision: 1,
    }));
  });
});
