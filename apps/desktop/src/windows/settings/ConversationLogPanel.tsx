import type { ConversationLogPageDto, ConversationMessageDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export function ConversationLogPanel() {
  const [messages, setMessages] = useState<ConversationMessageDto[]>([]);
  const [query, setQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState('');
  const [cursor, setCursor] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = async (beforeMessageId: number | null, append: boolean) => {
    setLoading(true);
    setError(null);
    try {
      const page = await invoke<ConversationLogPageDto>('list_conversation_log', {
        beforeMessageId,
        query: submittedQuery || null,
      });
      setMessages((current) => append ? [...page.messages, ...current] : page.messages);
      setCursor(page.next_before_message_id);
    } catch (problem) {
      setError(String(problem));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load(null, false);
  }, [submittedQuery]);

  return (
    <section className="log-panel" aria-label="会話ログ">
      <form
        className="log-toolbar"
        onSubmit={(event) => {
          event.preventDefault();
          setSubmittedQuery(query.trim());
        }}
      >
        <label>
          <span className="sr-only">会話ログを検索</span>
          <input
            value={query}
            maxLength={200}
            placeholder="会話ログを検索"
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <button type="submit">検索</button>
      </form>
      {error ? <p role="alert">{error}</p> : null}
      {messages.length === 0 && !loading ? <p className="empty-state">会話ログはありません。</p> : null}
      {cursor !== null ? (
        <button type="button" className="secondary-button load-older" disabled={loading} onClick={() => void load(cursor, true)}>
          古い履歴を読み込む
        </button>
      ) : null}
      <ol className="conversation-log-list">
        {messages.map((message) => (
          <li key={message.message_id} data-role={message.role}>
            <span className="log-meta">
              {message.role === 'user' ? 'あなた' : 'Parallel World'}
              {' · '}
              {new Date(message.created_at * 1_000).toLocaleString('ja-JP')}
            </span>
            <p>{message.text}</p>
          </li>
        ))}
      </ol>
    </section>
  );
}
