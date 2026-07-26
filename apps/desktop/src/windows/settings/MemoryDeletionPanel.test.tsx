import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { StrictMode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryDeletionPanel } from './MemoryDeletionPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const usage = {
  schema_version: 1,
  conversation_messages: 12,
  conversation_summaries: 1,
  long_term_memories: 3,
  tts_audio_files: 5,
  tts_audio_bytes: 1536,
};

async function waitForUsage(): Promise<void> {
  await screen.findByText(/12件のメッセージ \/ 1件の要約/);
}

describe('MemoryDeletionPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') return Promise.resolve(usage);
      if (command === 'get_retention_settings') {
        return Promise.resolve({ schema_version: 1, keep_messages: 30 });
      }
      if (command === 'set_retention_settings') {
        return Promise.resolve({ schema_version: 1, keep_messages: 42 });
      }
      if (command === 'delete_memories') {
        return Promise.resolve({
          schema_version: 1,
          deleted_records: 4,
          deleted_files: 0,
          freed_bytes: 0,
        });
      }
      if (command === 'clear_tts_audio_cache') {
        return Promise.resolve({
          schema_version: 1,
          deleted_records: 0,
          deleted_files: 5,
          freed_bytes: 1536,
        });
      }
      return Promise.resolve(null);
    });
  });

  it('saves the number of summarized messages to retain', async () => {
    render(<MemoryDeletionPanel />);
    const input = await screen.findByLabelText('保持するメッセージ件数');
    fireEvent.change(input, { target: { value: '42' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_retention_settings', {
      keepMessages: 42,
    }));
  });

  it('shows current destructive scopes and TTS cache size', async () => {
    render(<MemoryDeletionPanel />);

    expect(await screen.findByText(/12件のメッセージ \/ 1件の要約/)).toBeInTheDocument();
    expect(screen.getByText(/1件の要約 \/ 3件の長期記憶/)).toBeInTheDocument();
    expect(screen.getByText(/5ファイル \/ 1.5 KB/)).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('get_data_usage');
  });

  it('disables destructive actions while usage is loading', async () => {
    let resolveUsage: ((value: typeof usage) => void) | undefined;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') {
        return new Promise<typeof usage>((resolve) => {
          resolveUsage = resolve;
        });
      }
      return Promise.resolve(null);
    });
    render(<MemoryDeletionPanel />);

    const memoryButton = screen.getByRole('button', { name: '記憶を削除' });
    expect(memoryButton).toBeDisabled();
    expect(screen.getByRole('status')).toHaveTextContent('現在の使用量を確認しています');
    fireEvent.click(memoryButton);
    expect(screen.queryByLabelText('確認用テキスト')).not.toBeInTheDocument();

    await act(async () => resolveUsage?.(usage));
    await waitFor(() => expect(memoryButton).toBeEnabled());
  });

  it('keeps the latest StrictMode usage result when requests resolve out of order', async () => {
    const usageResolvers: Array<(value: typeof usage) => void> = [];
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') {
        return new Promise<typeof usage>((resolve) => usageResolvers.push(resolve));
      }
      return Promise.resolve(null);
    });
    render(
      <StrictMode>
        <MemoryDeletionPanel />
      </StrictMode>,
    );

    await waitFor(() => expect(usageResolvers).toHaveLength(2));
    await act(async () => {
      usageResolvers[1]?.({ ...usage, conversation_messages: 24 });
    });
    expect(await screen.findByText(/24件のメッセージ \/ 1件の要約/)).toBeInTheDocument();

    await act(async () => {
      usageResolvers[0]?.({ ...usage, conversation_messages: 99 });
    });
    await waitFor(() => {
      expect(screen.getByText(/24件のメッセージ \/ 1件の要約/)).toBeInTheDocument();
      expect(screen.queryByText(/99件のメッセージ \/ 1件の要約/)).not.toBeInTheDocument();
    });
  });

  it('keeps deletion disabled after a usage failure and supports retry', async () => {
    let attempts = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') {
        attempts += 1;
        return attempts === 1 ? Promise.reject(new Error('unavailable')) : Promise.resolve(usage);
      }
      return Promise.resolve(null);
    });
    render(<MemoryDeletionPanel />);

    const memoryButton = screen.getByRole('button', { name: '記憶を削除' });
    expect(await screen.findByRole('alert')).toHaveTextContent('削除操作は無効です');
    expect(memoryButton).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: '使用量を再取得' }));

    await waitForUsage();
    expect(memoryButton).toBeEnabled();
    expect(attempts).toBe(2);
  });

  it('requires the exact typed phrase before deleting memories', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm');
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    const memoryButton = screen.getByRole('button', { name: '記憶を削除' });
    fireEvent.click(memoryButton);

    const finalButton = screen.getByRole('button', { name: '完全に削除する' });
    expect(finalButton).toBeDisabled();
    expect(invokeMock).not.toHaveBeenCalledWith('delete_memories');
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '記憶を削除 ' },
    });
    expect(finalButton).toBeDisabled();
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '記憶を削除' },
    });
    fireEvent.click(finalButton);

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('delete_memories'));
    const dangerZone = screen.getByRole('region', { name: '削除' });
    expect(await within(dangerZone).findByText(/4件の記憶データ/)).toHaveAttribute(
      'role',
      'status',
    );
    await waitFor(() => expect(memoryButton).toHaveFocus());
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it('cancels a destructive confirmation, restores focus, and does not invoke the backend', async () => {
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    const historyButton = screen.getByRole('button', { name: '履歴を削除' });
    fireEvent.click(historyButton);
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '履歴を削除' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'キャンセル' }));

    expect(screen.queryByLabelText('確認用テキスト')).not.toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith('delete_conversation_history');
    await waitFor(() => expect(historyButton).toHaveFocus());
  });

  it('clears only the TTS audio cache after its own confirmation phrase', async () => {
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    fireEvent.click(screen.getByRole('button', { name: '音声キャッシュを削除' }));
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '音声を削除' },
    });
    fireEvent.click(screen.getByRole('button', { name: '完全に削除する' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('clear_tts_audio_cache'));
    const dangerZone = screen.getByRole('region', { name: '削除' });
    expect(await within(dangerZone).findByText(/5件の音声ファイル（1.5 KB）/)).toHaveAttribute(
      'role',
      'status',
    );
  });

  it('shows destructive command failures as alerts and keeps confirmation open', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') return Promise.resolve(usage);
      if (command === 'delete_memories') return Promise.reject(new Error('busy'));
      return Promise.resolve(null);
    });
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    fireEvent.click(screen.getByRole('button', { name: '記憶を削除' }));
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '記憶を削除' },
    });
    fireEvent.click(screen.getByRole('button', { name: '完全に削除する' }));

    const dangerZone = screen.getByRole('region', { name: '削除' });
    expect(await within(dangerZone).findByRole('alert')).toHaveTextContent('busy');
    expect(screen.getByLabelText('確認用テキスト')).toBeInTheDocument();
    await waitFor(() => {
      expect(invokeMock.mock.calls.filter(([command]) => command === 'get_data_usage')).toHaveLength(2);
    });
  });

  it('keeps final deletion disabled when the command and usage refresh both fail', async () => {
    let usageAttempts = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') {
        usageAttempts += 1;
        return usageAttempts === 1
          ? Promise.resolve(usage)
          : Promise.reject(new Error('usage unavailable'));
      }
      if (command === 'delete_memories') return Promise.reject(new Error('busy'));
      return Promise.resolve(null);
    });
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    fireEvent.click(screen.getByRole('button', { name: '記憶を削除' }));
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '記憶を削除' },
    });
    const finalButton = screen.getByRole('button', { name: '完全に削除する' });
    fireEvent.click(finalButton);

    expect(await screen.findByText(/削除操作は無効です/)).toBeInTheDocument();
    await waitFor(() => expect(finalButton).toBeDisabled());
    fireEvent.click(finalButton);
    expect(invokeMock.mock.calls.filter(([command]) => command === 'delete_memories')).toHaveLength(1);
  });

  it('focuses usage retry when deletion succeeds but refreshing usage fails', async () => {
    let usageAttempts = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_data_usage') {
        usageAttempts += 1;
        return usageAttempts === 1
          ? Promise.resolve(usage)
          : Promise.reject(new Error('usage unavailable'));
      }
      if (command === 'delete_memories') {
        return Promise.resolve({
          schema_version: 1,
          deleted_records: 4,
          deleted_files: 0,
          freed_bytes: 0,
        });
      }
      return Promise.resolve(null);
    });
    render(<MemoryDeletionPanel />);
    await waitForUsage();
    fireEvent.click(screen.getByRole('button', { name: '記憶を削除' }));
    fireEvent.change(screen.getByLabelText('確認用テキスト'), {
      target: { value: '記憶を削除' },
    });
    fireEvent.click(screen.getByRole('button', { name: '完全に削除する' }));

    const retryButton = await screen.findByRole('button', { name: '使用量を再取得' });
    await waitFor(() => expect(retryButton).toHaveFocus());
    expect(screen.queryByLabelText('確認用テキスト')).not.toBeInTheDocument();
  });
});
