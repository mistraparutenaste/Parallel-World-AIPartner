import type { TranscriptEventDto } from '@parallel-world/contracts';
import { useEffect, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';

/**
 * Conversation history and text input window.
 *
 * Recognized speech arrives as `stt-transcript` events and is
 * appended to the history. The history region is a polite live
 * region so appended messages are announced without stealing focus.
 * The stop button is always visible so speech can be interrupted at
 * any time.
 */
export function ChatWindow() {
  const [messages, setMessages] = useState<string[]>([]);

  useEffect(
    () =>
      subscribeEvent<TranscriptEventDto>('stt-transcript', (payload) => {
        setMessages((current) => [...current, payload.text]);
      }),
    [],
  );

  return (
    <main aria-label="チャット">
      <section aria-live="polite" aria-label="会話履歴">
        {messages.length === 0 ? (
          <p>まだメッセージはありません。</p>
        ) : (
          <ul>
            {messages.map((message, index) => (
              // History is append-only, so the index is stable.
              // eslint-disable-next-line react/no-array-index-key
              <li key={index}>{message}</li>
            ))}
          </ul>
        )}
      </section>
      <form>
        <label htmlFor="chat-message">メッセージ</label>
        <textarea id="chat-message" name="message" rows={2} />
        <button type="submit">送信</button>
        <button type="button">停止</button>
      </form>
    </main>
  );
}
