import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SelfReviewPanel } from './SelfReviewPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

describe('SelfReviewPanel', () => {
  beforeEach(() => invokeMock.mockReset());

  it('renders a read-only review and supports manual regeneration', async () => {
    invokeMock
      .mockResolvedValueOnce({ content: '以前の振り返り', generated_at: 1, source_message_id: 1 })
      .mockResolvedValueOnce({ content: '新しい振り返り', generated_at: 2, source_message_id: 2 });
    render(<SelfReviewPanel />);
    expect(await screen.findByText('以前の振り返り')).toBeInTheDocument();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '振り返りを更新' }));
    expect(await screen.findByText('新しい振り返り')).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenLastCalledWith('regenerate_self_review'));
  });
});
