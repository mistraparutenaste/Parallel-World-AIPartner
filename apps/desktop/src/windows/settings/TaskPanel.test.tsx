import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TaskPanel } from './TaskPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const center = (commitments: unknown[]) => ({
  schema_version: 1, domains: [], memories: [], pending: [], commitments,
  dialogue: null, temporary: false, temporary_revision: 0,
});

describe('TaskPanel', () => {
  beforeEach(() => invokeMock.mockReset());

  it('creates and renders a shared task', async () => {
    invokeMock
      .mockResolvedValueOnce(center([]))
      .mockResolvedValueOnce(center([{ id: 1, content: '買い物', status: 'open', due_at: null, revision: 1 }]));
    render(<TaskPanel />);
    const input = await screen.findByLabelText('新しいタスク');
    fireEvent.change(input, { target: { value: '買い物' } });
    fireEvent.click(screen.getByRole('button', { name: '追加' }));
    expect(await screen.findByText('買い物')).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenLastCalledWith('create_commitment', {
      content: '買い物', dueAt: null,
    }));
  });
});
