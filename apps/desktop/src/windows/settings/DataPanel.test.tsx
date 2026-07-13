import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { DataPanel } from './DataPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

describe('DataPanel', () => {
  beforeEach(() => { invokeMock.mockReset(); invokeMock.mockResolvedValue(null); });

  it('does not delete history when confirmation is declined', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(<DataPanel />);
    fireEvent.click(screen.getByRole('button', { name: '会話履歴を削除' }));
    expect(invokeMock).not.toHaveBeenCalledWith('delete_conversation_history');
  });

  it('deletes memories only after confirmation', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(<DataPanel />);
    fireEvent.click(screen.getByRole('button', { name: '記憶を削除' }));
    expect(invokeMock).toHaveBeenCalledWith('delete_memories');
  });

  it('exports only to the explicitly entered path', () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    render(<DataPanel />);
    fireEvent.change(screen.getByLabelText('保存先'), { target: { value: 'C:/backup/user-data.sqlite3' } });
    fireEvent.click(screen.getByRole('button', { name: 'エクスポート' }));
    expect(invokeMock).toHaveBeenCalledWith('export_user_data', { destination: 'C:/backup/user-data.sqlite3', allowOverwrite: false });
  });

  it('shows command failures as alerts', async () => {
    invokeMock.mockRejectedValueOnce(new Error('busy'));
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(<DataPanel />); fireEvent.click(screen.getByRole('button', { name: '記憶を削除' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('busy');
  });
});
