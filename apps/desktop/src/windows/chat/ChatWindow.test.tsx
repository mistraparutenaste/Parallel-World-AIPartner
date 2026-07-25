import type {
  ChatMessageEventDto,
  ConversationStateEventDto,
  DarkExpressionSafetyChangedEventDto,
  SafewordTriggeredEventDto,
  TtsStateEventDto,
} from '@parallel-world/contracts';
import {
  DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
  SAFEWORD_TRIGGERED_EVENT,
} from '@parallel-world/contracts';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
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

function fireSafetyChanged(payload: DarkExpressionSafetyChangedEventDto) {
  act(() => {
    listenHandlers.get(DARK_EXPRESSION_SAFETY_CHANGED_EVENT)?.({ payload });
  });
}

function fireSafewordTriggered(payload: SafewordTriggeredEventDto) {
  act(() => {
    listenHandlers.get(SAFEWORD_TRIGGERED_EVENT)?.({ payload });
  });
}

describe('ChatWindow', () => {
  it('clears displayed messages after durable history deletion', async () => {
    render(<ChatWindow />);
    fireMessage({ schema_version: 1, turn_id: 1, role: 'user', text: '消える' });
    expect(await screen.findByText(/消える/)).toBeInTheDocument();
    act(() => listenHandlers.get('conversation-history-deleted')?.({ payload: { schema_version: 1 } }));
    expect(screen.queryByText(/消える/)).not.toBeInTheDocument();
  });
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
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

  it('applies theme changes from shared UI preferences', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_ui_preferences') {
        return Promise.resolve({ schema_version: 1, theme: 'dark', chat_placement: 'popped' });
      }
      if (command === 'list_conversation_history') return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<ChatWindow />);
    await waitFor(() => expect(document.documentElement).toHaveAttribute('data-theme', 'dark'));
    act(() => document.documentElement.removeAttribute('data-theme'));
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

  it('shows history load failure without exposing technical detail', async () => {
    invokeMock.mockRejectedValueOnce(new Error('sqlite unavailable'));
    render(<ChatWindow />);
    expect(await screen.findByRole('alert')).toHaveTextContent('前の会話をうまく思い出せないみたい');
    expect(screen.queryByText(/sqlite unavailable/)).not.toBeInTheDocument();
    expect(screen.getByLabelText('メッセージ')).toBeEnabled();
  });

  it('sends with Enter without rendering send or stop buttons', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    fireEvent.change(screen.getByLabelText('メッセージ'), {
      target: { value: '今日の予定は?' },
    });
    fireEvent.keyDown(screen.getByLabelText('メッセージ'), {
      key: 'Enter',
      shiftKey: false,
    });

    expect(invokeMock).toHaveBeenCalledWith('send_chat_message', {
      text: '今日の予定は?',
    });
    expect(screen.getByLabelText('メッセージ')).toHaveValue('');
    expect(screen.queryByRole('button', { name: '送信' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '停止' })).not.toBeInTheDocument();
  });

  it('keeps Shift+Enter for line breaks', async () => {
    render(<ChatWindow />);
    await act(async () => {});
    const input = screen.getByLabelText('メッセージ');
    fireEvent.change(input, { target: { value: '一行目' } });
    fireEvent.keyDown(input, { key: 'Enter', shiftKey: true });
    expect(invokeMock).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
    expect(input).toHaveValue('一行目');
  });

  it('cancels generation with Escape', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    fireEvent.keyDown(screen.getByLabelText('メッセージ'), { key: 'Escape' });
    expect(invokeMock).toHaveBeenCalledWith('cancel_turn');
  });

  it('shows a quiet three-dot waiting state', async () => {
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

    expect(screen.getByRole('status')).toHaveAccessibleName('返答を待っています');
    expect(screen.getByRole('status').querySelectorAll('[aria-hidden="true"]')).toHaveLength(3);
  });

  it('surfaces tts degradation in natural language without touching the history', async () => {
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
      '声がうまく出ないみたい。文字では話せるよ',
    );
    expect(screen.queryByText(/tts request failed/)).not.toBeInTheDocument();
  });

  it('shows a persistent safety pause independently from ordinary notices', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_conversation_history') return Promise.resolve([]);
      if (command === 'get_dark_expression_safety_settings') {
        return Promise.resolve({
          schema_version: 1,
          safe_word: 'ストップ',
          dark_expression_paused: true,
        });
      }
      return Promise.resolve(null);
    });

    render(<ChatWindow />);

    expect(
      await screen.findByText('セーフワードを受け付けました。ダーク表現を停止しています。'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'ダーク表現を再開' })).toBeEnabled();

    act(() => {
      listenHandlers.get('tts-state')?.({
        payload: {
          schema_version: 1,
          available: false,
          message: 'tts request failed',
        } satisfies TtsStateEventDto,
      });
    });

    expect(screen.getByText('声がうまく出ないみたい。文字では話せるよ')).toBeInTheDocument();
    expect(
      screen.getByText('セーフワードを受け付けました。ダーク表現を停止しています。'),
    ).toBeInTheDocument();
  });

  it('stops the waiting state immediately and warns when the pause was not persisted', async () => {
    render(<ChatWindow />);
    await act(async () => {});

    act(() => {
      listenHandlers.get('conversation-state')?.({
        payload: {
          schema_version: 1,
          state: 'thinking',
          message: null,
        } satisfies ConversationStateEventDto,
      });
    });
    expect(screen.getByRole('status', { name: '返答を待っています' })).toBeInTheDocument();

    fireSafewordTriggered({
      schema_version: 1,
      pause_persisted: false,
    });

    expect(screen.queryByRole('status', { name: '返答を待っています' })).not.toBeInTheDocument();
    expect(
      screen.getByText('セーフワードを受け付けました。ダーク表現を停止しています。'),
    ).toBeInTheDocument();
    expect(
      screen.getByText('停止状態を保存できませんでした。アプリを閉じるまでは停止を保ちます。'),
    ).toBeInTheDocument();
  });

  it('resumes dark expression without sending a chat message or starting speech', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'list_conversation_history') return Promise.resolve([]);
      if (command === 'get_dark_expression_safety_settings') {
        return Promise.resolve({
          schema_version: 1,
          safe_word: 'ストップ',
          dark_expression_paused: true,
        });
      }
      if (command === 'resume_dark_expression') {
        return Promise.resolve({
          schema_version: 1,
          safe_word: 'ストップ',
          dark_expression_paused: false,
        });
      }
      return Promise.resolve(null);
    });

    render(<ChatWindow />);
    fireSafetyChanged({
      schema_version: 1,
      settings: {
        schema_version: 1,
        safe_word: 'ストップ',
        dark_expression_paused: true,
      },
    });
    fireEvent.click(await screen.findByRole('button', { name: 'ダーク表現を再開' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('resume_dark_expression'));
    await waitFor(() =>
      expect(
        screen.queryByText('セーフワードを受け付けました。ダーク表現を停止しています。'),
      ).not.toBeInTheDocument(),
    );
    expect(invokeMock).not.toHaveBeenCalledWith('send_chat_message', expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith('set_speech_playback', expect.anything());
  });
});
