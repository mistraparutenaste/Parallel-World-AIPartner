import type {
  ChatMessageEventDto,
  ConversationHistoryDeletedEventDto,
  ConversationMessageDto,
  ConversationStateDto,
  ConversationStateEventDto,
  DarkExpressionSafetyChangedEventDto,
  DarkExpressionSafetySettingsDto,
  SafewordTriggeredEventDto,
  TtsStateEventDto,
  UiPreferencesDto,
} from '@parallel-world/contracts';
import {
  CHAT_MESSAGE_EVENT,
  CONVERSATION_HISTORY_DELETED_EVENT,
  CONVERSATION_STATE_EVENT,
  DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
  DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
  SAFEWORD_TRIGGERED_EVENT,
  TTS_STATE_EVENT,
  UI_PREFERENCES_CHANGED_EVENT,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { applyThemePreference } from '../../shared/ui-preferences';

type DisplayMessage = {
  id: string;
  role: 'user' | 'assistant';
  text: string;
  turnId?: number;
};

const WAITING_STATES = new Set<ConversationStateDto>(['starting', 'thinking', 'recovering']);

function naturalFailure(state: ConversationStateDto) {
  if (state === 'tts_unavailable') {
    return '声がうまく出ないみたい。文字では話せるよ';
  }
  if (state === 'stt_unavailable') {
    return 'うまく聞き取れないみたい。文字で話してくれる？';
  }
  if (state === 'llm_unavailable') {
    return '今はうまく返事ができないみたい。少し待って、もう一度話しかけて';
  }
  return null;
}

/**
 * Conversation history and a keyboard-first composer.
 *
 * The management shell can keep this component mounted while another
 * surface is visible by setting `inactive`. Persistent episode navigation
 * is intentionally owned by the future episode boundary, not this view.
 */
export function ChatWindow({ inactive = false }: { inactive?: boolean }) {
  const [messages, setMessages] = useState<DisplayMessage[]>([]);
  const [draft, setDraft] = useState('');
  const [waiting, setWaiting] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [safety, setSafety] = useState<DarkExpressionSafetySettingsDto | null>(null);
  const [safetyPersistenceWarning, setSafetyPersistenceWarning] = useState(false);
  const [resumingDarkExpression, setResumingDarkExpression] = useState(false);

  useEffect(() => {
    let mounted = true;
    const stopPreferences = subscribeEvent<UiPreferencesDto>(
      UI_PREFERENCES_CHANGED_EVENT,
      (preferences) => applyThemePreference(preferences.theme),
    );
    const stopMessages = subscribeEvent<ChatMessageEventDto>(
      CHAT_MESSAGE_EVENT,
      (payload) => {
        setWaiting(false);
        setMessages((current) => {
          const id = payload.message_id == null
            ? `live:${payload.turn_id}:${payload.role}`
            : `db:${payload.message_id}`;
          const existing = current.findIndex((message) => message.id === id);
          if (existing >= 0) {
            if (payload.role !== 'assistant') return current;
            const next = [...current];
            const prior = next[existing]!;
            next[existing] = { ...prior, text: prior.text + payload.text };
            return next;
          }
          return [
            ...current,
            {
              id,
              turnId: payload.turn_id,
              role: payload.role,
              text: payload.text,
            },
          ];
        });
      },
    );

    invoke<ConversationMessageDto[]>('list_conversation_history')
      .then((history) => {
        if (!Array.isArray(history)) return;
        setMessages((current) => {
          const liveTurns = new Set(
            current
              .filter((message) => message.turnId != null)
              .map((message) => `${message.turnId}:${message.role}`),
          );
          const durable = history
            .filter((message) => !liveTurns.has(`${message.turn_id}:${message.role}`))
            .map((message) => ({
              id: `db:${message.message_id}`,
              turnId: message.turn_id ?? undefined,
              role: message.role,
              text: message.text,
            }));
          return [...durable, ...current];
        });
      })
      .catch(() => setNotice('前の会話をうまく思い出せないみたい。ここからは話せるよ'));

    invoke<UiPreferencesDto>('get_ui_preferences')
      .then((preferences) => {
        if (mounted && preferences) applyThemePreference(preferences.theme);
      })
      .catch(() => {});

    const stopState = subscribeEvent<ConversationStateEventDto>(
      CONVERSATION_STATE_EVENT,
      (payload) => {
        setWaiting(WAITING_STATES.has(payload.state));
        const failure = naturalFailure(payload.state);
        if (failure) setNotice(failure);
        if (payload.state === 'idle' || payload.state === 'cancelled') {
          setWaiting(false);
        }
      },
    );

    const stopTts = subscribeEvent<TtsStateEventDto>(TTS_STATE_EVENT, (payload) => {
      if (!payload.available) {
        setNotice('声がうまく出ないみたい。文字では話せるよ');
      }
    });
    const stopDeleted = subscribeEvent<ConversationHistoryDeletedEventDto>(
      CONVERSATION_HISTORY_DELETED_EVENT,
      () => setMessages([]),
    );
    const stopSafetyChanged = subscribeEvent<DarkExpressionSafetyChangedEventDto>(
      DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
      (payload) => {
        setSafety(payload.settings);
        if (!payload.settings.dark_expression_paused) {
          setSafetyPersistenceWarning(false);
        }
      },
    );
    const stopSafewordTriggered = subscribeEvent<SafewordTriggeredEventDto>(
      SAFEWORD_TRIGGERED_EVENT,
      (payload) => {
        setWaiting(false);
        setSafety((current) => ({
          schema_version: current?.schema_version ?? DARK_EXPRESSION_SAFETY_SCHEMA_VERSION,
          safe_word: current?.safe_word ?? null,
          dark_expression_paused: true,
        }));
        setSafetyPersistenceWarning(!payload.pause_persisted);
      },
    );

    invoke<DarkExpressionSafetySettingsDto>('get_dark_expression_safety_settings')
      .then((settings) => {
        if (!mounted || !settings) return;
        setSafety((current) => (
          current?.dark_expression_paused && !settings.dark_expression_paused
            ? current
            : settings
        ));
      })
      .catch(() => {});

    return () => {
      mounted = false;
      stopPreferences();
      stopMessages();
      stopState();
      stopTts();
      stopDeleted();
      stopSafetyChanged();
      stopSafewordTriggered();
    };
  }, []);

  const send = () => {
    const text = draft.trim();
    if (text === '') return;
    setDraft('');
    setNotice(null);
    setWaiting(true);
    invoke('send_chat_message', { text }).catch(() => {
      setWaiting(false);
      setNotice('今はうまく返事ができないみたい。少し待って、もう一度話しかけて');
    });
  };

  const stop = () => {
    setWaiting(false);
    invoke('cancel_turn').catch(() => {});
  };

  const resumeDarkExpression = () => {
    if (resumingDarkExpression) return;
    setResumingDarkExpression(true);
    invoke<DarkExpressionSafetySettingsDto>('resume_dark_expression')
      .then((settings) => {
        setSafety(settings);
        setSafetyPersistenceWarning(false);
      })
      .catch(() => {
        setNotice('ダーク表現を再開できませんでした。設定からもう一度試してください。');
      })
      .finally(() => setResumingDarkExpression(false));
  };

  return (
    <section aria-label="チャット" className="chat-surface">
      <section className="chat-history" aria-live="polite" aria-label="会話履歴">
        {messages.length > 0 ? (
          <ul>
            {messages.map((message, index) => (
              <li
                key={message.id}
                data-role={message.role}
                data-latest={index >= Math.max(0, messages.length - 2)}
              >
                {message.text}
              </li>
            ))}
          </ul>
        ) : null}

        {waiting ? (
          <p className="conversation-waiting" role="status" aria-label="返答を待っています">
            <span aria-hidden="true" />
            <span aria-hidden="true" />
            <span aria-hidden="true" />
          </p>
        ) : null}

        {safety?.dark_expression_paused ? (
          <div className="conversation-safety-notice" role="alert">
            <p>セーフワードを受け付けました。ダーク表現を停止しています。</p>
            {safetyPersistenceWarning ? (
              <p className="conversation-safety-warning">
                停止状態を保存できませんでした。アプリを閉じるまでは停止を保ちます。
              </p>
            ) : null}
            <button
              type="button"
              disabled={resumingDarkExpression}
              onClick={resumeDarkExpression}
            >
              {resumingDarkExpression ? '再開しています…' : 'ダーク表現を再開'}
            </button>
          </div>
        ) : null}

        {notice ? <p className="conversation-notice" role="alert">{notice}</p> : null}
      </section>

      <form
        className="chat-composer"
        onSubmit={(event) => {
          event.preventDefault();
          send();
        }}
      >
        <label className="sr-only" htmlFor="chat-message">メッセージ</label>
        <textarea
          id="chat-message"
          name="message"
          rows={1}
          value={draft}
          disabled={inactive}
          placeholder="話しかける…"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              stop();
            } else if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault();
              send();
            }
          }}
        />
      </form>
    </section>
  );
}
