import type { ConversationStateDto } from '@parallel-world/contracts';

const DEFAULT_TIMEOUT_SECONDS = 20;
const MIN_TIMEOUT_SECONDS = 10;
const MAX_TIMEOUT_SECONDS = 600;

const ACTIVE_CONVERSATION_STATES: ReadonlySet<ConversationStateDto> = new Set([
  'listening',
  'transcribing',
  'thinking',
  'speaking',
  'interrupting',
]);

type TimerHandle = unknown;

type VisibilityTarget = {
  addEventListener(type: string, listener: EventListener): void;
  removeEventListener(type: string, listener: EventListener): void;
};

export type CharacterIdleResetDependencies = {
  now: () => number;
  setTimer: (callback: () => void, delayMs: number) => TimerHandle;
  clearTimer: (handle: TimerHandle) => void;
  visibilityTarget?: VisibilityTarget | undefined;
};

function defaultDependencies(): CharacterIdleResetDependencies {
  return {
    now: () => performance.now(),
    setTimer: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
    clearTimer: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
    visibilityTarget: typeof document === 'undefined' ? undefined : document,
  };
}

/**
 * Owns the single deadline that returns a character to its default expression.
 * Conversation and audio activity are deliberately independent from rendering,
 * so React can wire this owner to either renderer implementation.
 */
export class CharacterIdleResetController {
  private readonly resetDefaultExpression: () => void;
  private readonly dependencies: CharacterIdleResetDependencies;
  private readonly onVisibilityChange: EventListener;
  private timeoutSeconds: number | null = DEFAULT_TIMEOUT_SECONDS;
  private lastActivityMs: number;
  private conversationActive = false;
  private audioActive = false;
  private resetHandled = false;
  private timer: TimerHandle | null = null;
  private disposed = false;

  constructor(
    resetDefaultExpression: () => void,
    dependencies: CharacterIdleResetDependencies = defaultDependencies(),
  ) {
    this.resetDefaultExpression = resetDefaultExpression;
    this.dependencies = dependencies;
    this.lastActivityMs = dependencies.now();
    this.onVisibilityChange = () => this.schedule();
    dependencies.visibilityTarget?.addEventListener(
      'visibilitychange',
      this.onVisibilityChange,
    );
    this.schedule();
  }

  activity(): void {
    if (this.disposed) return;
    this.lastActivityMs = this.dependencies.now();
    this.resetHandled = false;
    this.schedule();
  }

  setConversationState(state: ConversationStateDto): void {
    if (this.disposed) return;
    this.conversationActive = ACTIVE_CONVERSATION_STATES.has(state);
    this.lastActivityMs = this.dependencies.now();
    this.resetHandled = false;
    this.schedule();
  }

  setAudioActive(active: boolean): void {
    if (this.disposed || this.audioActive === active) return;
    this.audioActive = active;
    this.lastActivityMs = this.dependencies.now();
    this.resetHandled = false;
    this.schedule();
  }

  setTimeoutSeconds(value: number | null): void {
    if (
      value !== null
      && (!Number.isInteger(value)
        || value < MIN_TIMEOUT_SECONDS
        || value > MAX_TIMEOUT_SECONDS)
    ) {
      throw new RangeError('idle expression timeout must be null or 10..600 seconds');
    }
    if (this.disposed) return;
    this.timeoutSeconds = value;
    this.schedule();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.cancelTimer();
    this.dependencies.visibilityTarget?.removeEventListener(
      'visibilitychange',
      this.onVisibilityChange,
    );
  }

  private schedule(): void {
    this.cancelTimer();
    if (
      this.disposed
      || this.timeoutSeconds === null
      || this.conversationActive
      || this.audioActive
      || this.resetHandled
    ) {
      return;
    }

    const deadlineMs = this.lastActivityMs + this.timeoutSeconds * 1_000;
    const remainingMs = deadlineMs - this.dependencies.now();
    if (remainingMs <= 0) {
      this.resetHandled = true;
      this.resetDefaultExpression();
      return;
    }

    this.timer = this.dependencies.setTimer(() => {
      this.timer = null;
      this.schedule();
    }, remainingMs);
  }

  private cancelTimer(): void {
    if (this.timer === null) return;
    this.dependencies.clearTimer(this.timer);
    this.timer = null;
  }
}
