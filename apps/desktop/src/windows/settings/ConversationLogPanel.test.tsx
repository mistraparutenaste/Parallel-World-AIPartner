import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ConversationLogPanel } from './ConversationLogPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const stylesheet = resolve(process.cwd(), 'src/shared/styles/global.css');
const longUnbrokenMessage = 'PW'.repeat(250);

describe('ConversationLogPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({
      schema_version: 1,
      messages: [{
        schema_version: 1,
        message_id: 1,
        turn_id: 1,
        role: 'user',
        text: longUnbrokenMessage,
        created_at: 1_700_000_000,
      }],
      next_before_message_id: null,
    });
  });

  afterEach(() => {
    document.head.querySelector('[data-conversation-log-styles]')?.remove();
  });

  it('keeps an unbroken message from widening the settings panel and toolbar', async () => {
    const style = document.createElement('style');
    style.dataset.conversationLogStyles = '';
    style.textContent = await readFile(stylesheet, 'utf8');
    document.head.append(style);

    render(
      <main className="control-center" data-ui-style="conversation-first">
        <div className="settings-body">
          <div className="panel-stack">
            <ConversationLogPanel />
          </div>
        </div>
      </main>,
    );

    const message = await screen.findByText(longUnbrokenMessage);

    expect(getComputedStyle(message).overflowWrap).toBe('anywhere');
    expect(screen.getByRole('button', { name: '検索' })).toBeInTheDocument();
  });
});
