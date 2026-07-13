import type {
  ChatMessageEventDto,
  ConversationStateEventDto,
  TtsStateEventDto,
} from '@parallel-world/contracts';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatWindow } from './ChatWindow';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));
const listenHandlers = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: (name: string, handler: (event: { payload: unknown }) => void) => {
      listenHandlers.set(name, handler);
      return Promise.resolve(() => {
        listenHandlers.delete(name);
      });
    },
  }),
}));

function fireMessage(payload: ChatMessageEventDto) {
  act(() => {
    listenHandlers.get('chat-message')?.({ payload });
  });
}

describe('ChatWindow', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
  });

  it('shows user and streamed assistant messages', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    fireMessage({
      schema_version: 1,
      turn_id: 1,
      role: 'user',
      text: 'こんにちは',
    });
    fireMessage({
      schema_version: 1,
      turn_id: 1,
      role: 'assistant',
      text: 'やあ、こんにちは。',
    });

    expect(screen.getByText(/こんにちは$/)).toBeInTheDocument();
    expect(screen.getByText('やあ、こんにちは。')).toBeInTheDocument();
  });

  it('loads persisted history and deduplicates a matching live event', async () => {
    invokeMock.mockResolvedValueOnce([{ schema_version: 1, message_id: 7, turn_id: 1, role: 'user', text: 'saved', created_at: 1 }]);
    render(<ChatWindow />);
    expect(await screen.findByText(/saved$/)).toBeInTheDocument();
    fireMessage({ schema_version: 1, message_id: 7, turn_id: 1, role: 'user', text: 'saved' });
    expect(screen.getAllByText(/saved$/)).toHaveLength(1);
  });

  it('keeps identical text from different turns', async () => {
    invokeMock.mockResolvedValueOnce([]);
    render(<ChatWindow />); await act(async () => {});
    fireMessage({ schema_version: 1, turn_id: 1, role: 'user', text: 'same' });
    fireMessage({ schema_version: 1, turn_id: 2, role: 'user', text: 'same' });
    expect(screen.getAllByText(/same$/)).toHaveLength(2);
  });

  it('combines assistant sentence events for one turn', async () => {
    invokeMock.mockResolvedValueOnce([]);
    render(<ChatWindow />); await act(async () => {});
    fireMessage({ schema_version: 1, turn_id: 1, role: 'assistant', text: 'one' });
    fireMessage({ schema_version: 1, turn_id: 1, role: 'assistant', text: 'two' });
    expect(screen.getByText('onetwo')).toBeInTheDocument();
  });

  it('shows history load failure as a nonfatal alert', async () => {
    invokeMock.mockRejectedValueOnce(new Error('sqlite unavailable'));
    render(<ChatWindow />);
    expect(await screen.findByRole('alert')).toHaveTextContent('sqlite unavailable');
    expect(screen.getByLabelText('メッセージ')).toBeEnabled();
  });

  it('sends the draft through send_chat_message', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    fireEvent.change(screen.getByLabelText('メッセージ'), {
      target: { value: '今日の予定は?' },
    });
    fireEvent.click(screen.getByRole('button', { name: '送信' }));

    expect(invokeMock).toHaveBeenCalledWith('send_chat_message', {
      text: '今日の予定は?',
    });
    expect(screen.getByLabelText('メッセージ')).toHaveValue('');
  });

  it('cancels generation with the stop button', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    fireEvent.click(screen.getByRole('button', { name: '停止' }));

    expect(invokeMock).toHaveBeenCalledWith('cancel_turn');
  });

  it('shows thinking status from conversation-state events', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    const payload: ConversationStateEventDto = {
      schema_version: 1,
      state: 'thinking',
      message: null,
    };
    act(() => {
      listenHandlers.get('conversation-state')?.({ payload });
    });

    expect(screen.getByRole('status')).toHaveTextContent('考え中…');
  });

  it('surfaces tts degradation without touching the history', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    const payload: TtsStateEventDto = {
      schema_version: 1,
      available: false,
      message: 'tts request failed',
    };
    act(() => {
      listenHandlers.get('tts-state')?.({ payload });
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      '音声合成に接続できません: tts request failed',
    );
  });
});
