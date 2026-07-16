import { beforeEach, describe, expect, it, vi } from 'vitest';
import { subscribeEvent } from './event-bus';

const listenMock = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ listen: listenMock }),
}));

describe('subscribeEvent', () => {
  beforeEach(() => {
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it('becomes a safe no-op when rendered outside the Tauri runtime', () => {
    const handler = vi.fn();

    const unsubscribe = subscribeEvent('browser-preview-event', handler);

    expect(listenMock).not.toHaveBeenCalled();
    expect(() => unsubscribe()).not.toThrow();
  });

  it('keeps window-scoped subscriptions inside the Tauri runtime', () => {
    (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};

    subscribeEvent('tauri-runtime-event', vi.fn());

    expect(listenMock).toHaveBeenCalledWith(
      'tauri-runtime-event',
      expect.any(Function),
    );
  });
});
