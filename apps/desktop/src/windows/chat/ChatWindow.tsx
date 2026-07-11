import { useState, type FormEvent } from 'react';
import type { AppStatusDto } from '@parallel-world/contracts';
import { ActionButton } from '../../shared/components/ActionButton';
import { Icon } from '../../shared/components/Icons';
import { StatusBadge } from '../../shared/components/StatusBadge';
import { WindowFrame } from '../../shared/components/WindowFrame';
import '../../shared/styles/global.css';

export const initialAppStatus = {
  schema_version: 1,
  conversation_state: 'idle',
} satisfies AppStatusDto;

export function ChatWindow() {
  const [message, setMessage] = useState('');
  const [messages, setMessages] = useState<string[]>([]);
  const [appStatus, setAppStatus] = useState<AppStatusDto>(initialAppStatus);
  const isProcessing = appStatus.conversation_state !== 'idle';

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const nextMessage = message.trim();
    if (!nextMessage) return;
    setMessages((current) => [...current, nextMessage]);
    setMessage('');
    setAppStatus((current) => ({ ...current, conversation_state: 'thinking' }));
  }

  function handleStop() {
    setAppStatus((current) => ({ ...current, conversation_state: 'idle' }));
  }

  return (
    <WindowFrame title="Parallel World" status={<StatusBadge>待機中</StatusBadge>}>
      <section className="chat-layout">
        <div className="conversation-rail" aria-live="polite">
          {messages.length === 0 ? (
            <div className="empty-state">
              <Icon name="chat" className="empty-state__icon" />
              <h2>会話をはじめましょう</h2>
            </div>
          ) : (
            <ol className="message-list">
              {messages.map((item, index) => <li key={`${item}-${index}`}>{item}</li>)}
            </ol>
          )}
        </div>
        <form className="composer" onSubmit={handleSubmit}>
          <label className="sr-only" htmlFor="chat-message">メッセージ</label>
          <input id="chat-message" value={message} onChange={(event) => setMessage(event.target.value)} placeholder="メッセージ" />
          <ActionButton type="submit" variant="primary" disabled={!message.trim()}>送信</ActionButton>
          <ActionButton type="button" onClick={handleStop} disabled={!isProcessing}>停止</ActionButton>
        </form>
        <div className="status-rail" aria-label="処理状態">
          <div><Icon name="microphone" /><span>STT</span><StatusBadge>待機中</StatusBadge></div>
          <div><Icon name="model" /><span>LLM</span><StatusBadge>待機中</StatusBadge></div>
          <div><Icon name="speaker" /><span>TTS</span><StatusBadge>待機中</StatusBadge></div>
        </div>
      </section>
    </WindowFrame>
  );
}
