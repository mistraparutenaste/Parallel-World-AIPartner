import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryTransferPanel } from './MemoryTransferPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const usage = {
  schema_version: 1,
  conversation_messages: 3,
  conversation_summaries: 1,
  long_term_memories: 2,
  tts_audio_files: 0,
  tts_audio_bytes: 0,
};

describe('MemoryTransferPanel', () => {
  beforeEach(() => {
    invokeMock.mockImplementation((command: string) => (
      command === 'get_data_usage' ? Promise.resolve(usage) : Promise.resolve(null)
    ));
  });

  it('imports memory CSV from the selected path', async () => {
    render(<MemoryTransferPanel />);
    const source = await screen.findByLabelText('メモリーCSVの読込元');
    fireEvent.change(source, { target: { value: 'C:\\backup\\memories.csv' } });
    fireEvent.click(screen.getByRole('button', { name: 'CSVインポート' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('import_memories_csv', {
      source: 'C:\\backup\\memories.csv',
    }));
    expect(await screen.findByRole('status')).toHaveTextContent('インポートしました');
  });
});
