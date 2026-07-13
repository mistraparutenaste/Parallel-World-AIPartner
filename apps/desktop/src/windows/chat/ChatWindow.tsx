import type {
  ChatMessageEventDto,
  ConversationMessageDto,
  ConversationStateEventDto,
  TtsStateEventDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';

const STATE_LABELS: Partial<Record<ConversationStateEventDto['state'], string>> =
  {
    thinking: '考え中…',
    speaking: '返答中…',
    llm_unavailable: 'LLMに接続できません',
  };

type DisplayMessage = {
  id: string;
  role: 'user' | 'assistant';
  text: string;
};

/**
 * Conversation history and text input window.
 *
 * User messages (typed or recognized speech) and streamed assistant
 * sentences arrive as `chat-message` events; generation status
 * arrives as `conversation-state` events. The stop button cancels
 * the in-flight turn.
 */
export function ChatWindow() {
  const [messages, setMessages] = useState<DisplayMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [stateLabel, setStateLabel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<ConversationMessageDto[]>('list_conversation_history').then((history) => {
      if (!Array.isArray(history)) return;
      setMessages((current) => {
        const liveKeys = new Set(current.map((m) => `${m.role}:${m.text}`));
        return [...history.filter((m) => !liveKeys.has(`${m.role}:${m.text}`)).map((m) => ({ id: `db:${m.message_id}`, role: m.role, text: m.text })), ...current];
      });
    }).catch(() => {});
    const stopMessages = subscribeEvent<ChatMessageEventDto>(
      'chat-message',
      (payload) => {
        setMessages((current) => {
          const id = payload.message_id == null ? `live:${payload.turn_id}:${payload.role}:${payload.text}` : `db:${payload.message_id}`;
          return current.some((m) => m.id === id || (m.role === payload.role && m.text === payload.text)) ? current : [...current, { id, role: payload.role, text: payload.text }];
        });
      },
    );
    const stopState = subscribeEvent<ConversationStateEventDto>(
      'conversation-state',
      (payload) => {
        setStateLabel(STATE_LABELS[payload.state] ?? null);
        // Keep the last error detail visible; state-only events
        // (message: null) must not wipe it.
        if (payload.message != null) {
          setError(payload.message);
        }
      },
    );
    // TTS failures degrade to text-only; surface the reason without
    // interrupting the conversation (TTS障害 → テキスト表示).
    const stopTts = subscribeEvent<TtsStateEventDto>('tts-state', (payload) => {
      if (!payload.available) {
        setError(`音声合成に接続できません: ${payload.message ?? ''}`);
      }
    });
    return () => {
      stopMessages();
      stopState();
      stopTts();
    };
  }, []);

  const send = () => {
    const text = draft.trim();
    if (text === '') {
      return;
    }
    setDraft('');
    setError(null);
    invoke('send_chat_message', { text }).catch((problem: unknown) => {
      setError(String(problem));
    });
  };

  const stop = () => {
    invoke('cancel_turn').catch(() => {});
  };

  return (
    <main aria-label="チャット">
      <section aria-live="polite" aria-label="会話履歴">
        {messages.length === 0 ? (
          <p>まだメッセージはありません。</p>
        ) : (
          <ul>
            {messages.map((message) => (
              <li key={message.id} data-role={message.role}>
                {message.role === 'user' ? 'あなた: ' : ''}
                {message.text}
              </li>
            ))}
          </ul>
        )}
        {stateLabel !== null && <p role="status">{stateLabel}</p>}
        {error !== null && <p role="alert">{error}</p>}
      </section>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          send();
        }}
      >
        <label htmlFor="chat-message">メッセージ</label>
        <textarea
          id="chat-message"
          name="message"
          rows={2}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault();
              send();
            }
          }}
        />
        <button type="submit">送信</button>
        <button type="button" onClick={stop}>
          停止
        </button>
      </form>
    </main>
  );
}
