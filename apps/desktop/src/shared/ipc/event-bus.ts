import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

/**
 * Window-scoped event subscriptions with synchronous unsubscribe.
 *
 * `listen()` registers and unregisters asynchronously, so effect
 * cleanup can race with registration under React StrictMode's
 * double-mounted effects and leave a stray listener behind (events
 * then arrive twice). This bus keeps exactly one Tauri listener per
 * event name for the lifetime of the window and fans out to handlers
 * held in a Set, where add/remove are synchronous and race-free.
 */

type Handler = (payload: unknown) => void;

const channels = new Map<string, Set<Handler>>();

export function subscribeEvent<T>(
  eventName: string,
  handler: (payload: T) => void,
): () => void {
  if (
    typeof window === 'undefined'
    || !('__TAURI_INTERNALS__' in window)
  ) {
    return () => {};
  }

  let handlers = channels.get(eventName);
  if (!handlers) {
    const created = new Set<Handler>();
    channels.set(eventName, created);
    handlers = created;
    // Scope the listener to this window: default listeners use the
    // `Any` target and would also receive copies of events emitted
    // to *other* windows under the same event name.
    getCurrentWebviewWindow()
      .listen<T>(eventName, (event) => {
        for (const registered of created) {
          registered(event.payload);
        }
      })
      .catch((error: unknown) => {
        console.error(`failed to subscribe to ${eventName}`, error);
        channels.delete(eventName);
      });
  }
  const entry = handler as Handler;
  handlers.add(entry);
  return () => {
    handlers.delete(entry);
  };
}
