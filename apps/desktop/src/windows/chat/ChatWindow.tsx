/**
 * Conversation history and text input window.
 *
 * The history region is a polite live region so appended messages are
 * announced without stealing focus. The stop button is always visible
 * so speech can be interrupted at any time.
 */
export function ChatWindow() {
  return (
    <main aria-label="チャット">
      <section aria-live="polite" aria-label="会話履歴">
        <p>まだメッセージはありません。</p>
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
