import type { TranscriptEventDto } from '@parallel-world/contracts';
import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatWindow } from './ChatWindow';

const listenHandlers = vi.hoisted(
  () => new Map<string, (event: { payload: unknown }) => void>(),
);
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    listenHandlers.set(name, handler);
    return Promise.resolve(() => {
      listenHandlers.delete(name);
    });
  }),
}));

describe('ChatWindow transcripts', () => {
  beforeEach(() => {
    listenHandlers.clear();
  });

  it('appends recognized speech to the history', async () => {
    render(<ChatWindow />);
    expect(screen.getByText('まだメッセージはありません。')).toBeInTheDocument();

    // Wait for the subscription to be registered.
    await act(async () => {});
    const payload: TranscriptEventDto = {
      schema_version: 1,
      text: 'こんにちは',
    };
    act(() => {
      listenHandlers.get('stt-transcript')?.({ payload });
    });

    expect(screen.getByText('こんにちは')).toBeInTheDocument();
    expect(
      screen.queryByText('まだメッセージはありません。'),
    ).not.toBeInTheDocument();
  });
});
