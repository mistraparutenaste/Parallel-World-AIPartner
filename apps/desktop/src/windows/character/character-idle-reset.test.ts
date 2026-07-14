import type { ConversationStateDto } from '@parallel-world/contracts';
import { describe, expect, it, vi } from 'vitest';
import {
  CharacterIdleResetController,
  type CharacterIdleResetDependencies,
} from './character-idle-reset';

class FakeVisibilityTarget {
  private readonly listeners = new Set<EventListener>();

  addEventListener(type: string, listener: EventListener): void {
    if (type === 'visibilitychange') this.listeners.add(listener);
  }

  removeEventListener(type: string, listener: EventListener): void {
    if (type === 'visibilitychange') this.listeners.delete(listener);
  }

  wake(): void {
    for (const listener of [...this.listeners]) listener(new Event('visibilitychange'));
  }

  get listenerCount(): number {
    return this.listeners.size;
  }
}

class FakeClock {
  private nowMs = 0;
  private nextId = 1;
  private readonly timers = new Map<number, { at: number; callback: () => void }>();
  readonly visibility = new FakeVisibilityTarget();

  readonly dependencies: CharacterIdleResetDependencies = {
    now: () => this.nowMs,
    setTimer: (callback, delayMs) => {
      const id = this.nextId++;
      this.timers.set(id, { at: this.nowMs + delayMs, callback });
      return id;
    },
    clearTimer: (handle) => {
      this.timers.delete(handle as number);
    },
    visibilityTarget: this.visibility,
  };

  advance(ms: number): void {
    this.nowMs += ms;
    this.runDueTimers();
  }

  elapseWhileSuspended(ms: number): void {
    this.nowMs += ms;
  }

  get timerCount(): number {
    return this.timers.size;
  }

  private runDueTimers(): void {
    while (true) {
      const due = [...this.timers.entries()]
        .filter(([, timer]) => timer.at <= this.nowMs)
        .sort((left, right) => left[1].at - right[1].at)[0];
      if (!due) return;
      this.timers.delete(due[0]);
      due[1].callback();
    }
  }
}

function setup() {
  const clock = new FakeClock();
  const resetDefaultExpression = vi.fn();
  const controller = new CharacterIdleResetController(
    resetDefaultExpression,
    clock.dependencies,
  );
  return { clock, controller, resetDefaultExpression };
}

describe('CharacterIdleResetController', () => {
  it('resets after the default twenty-second idle period', () => {
    const { clock, resetDefaultExpression } = setup();

    clock.advance(19_999);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(1);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it.each<ConversationStateDto>([
    'listening',
    'transcribing',
    'thinking',
    'speaking',
    'interrupting',
  ])('does not reset while conversation state is %s', (state) => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(5_000);
    controller.setConversationState(state);
    clock.advance(60_000);

    expect(resetDefaultExpression).not.toHaveBeenCalled();
    expect(clock.timerCount).toBe(0);

    controller.setConversationState('idle');
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(19_999);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(1);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it('updates the deadline for explicit activity', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(15_000);
    controller.activity();
    clock.advance(19_999);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(1);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it('resets once per activity even across multiple idle periods', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(20_000);
    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);

    clock.advance(120_000);
    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);

    controller.activity();
    clock.advance(20_000);
    expect(resetDefaultExpression).toHaveBeenCalledTimes(2);
  });

  it('pauses while speech audio is active and resumes from the latest audio activity', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(5_000);
    controller.setAudioActive(true);
    clock.advance(30_000);
    expect(resetDefaultExpression).not.toHaveBeenCalled();

    controller.setAudioActive(false);
    clock.advance(19_999);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(1);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it('disables reset when the timeout is never', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    controller.setTimeoutSeconds(null);
    clock.advance(600_000);

    expect(resetDefaultExpression).not.toHaveBeenCalled();
    expect(clock.timerCount).toBe(0);
  });

  it('shortens from the last activity and resets immediately past the new deadline', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(11_000);
    controller.setTimeoutSeconds(10);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
    expect(clock.timerCount).toBe(0);
  });

  it('extends from the last activity without moving the activity timestamp', () => {
    const { clock, controller, resetDefaultExpression } = setup();

    clock.advance(15_000);
    controller.setTimeoutSeconds(30);
    clock.advance(14_999);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.advance(1);

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it('rechecks elapsed monotonic time when visibility wakes', () => {
    const { clock, resetDefaultExpression } = setup();

    clock.elapseWhileSuspended(25_000);
    expect(resetDefaultExpression).not.toHaveBeenCalled();
    clock.visibility.wake();

    expect(resetDefaultExpression).toHaveBeenCalledTimes(1);
  });

  it('owns at most one timer while deadlines are rescheduled', () => {
    const { clock, controller } = setup();

    controller.activity();
    controller.setTimeoutSeconds(30);
    controller.setConversationState('thinking');
    controller.setConversationState('idle');

    expect(clock.timerCount).toBe(1);
  });

  it('rejects timeout values outside the persisted range', () => {
    const { controller } = setup();

    expect(() => controller.setTimeoutSeconds(9)).toThrow(RangeError);
    expect(() => controller.setTimeoutSeconds(601)).toThrow(RangeError);
  });

  it('cancels its timer and visibility listener on dispose', () => {
    const { clock, controller, resetDefaultExpression } = setup();
    expect(clock.visibility.listenerCount).toBe(1);

    controller.dispose();
    controller.dispose();
    expect(clock.timerCount).toBe(0);
    expect(clock.visibility.listenerCount).toBe(0);

    clock.advance(30_000);
    clock.visibility.wake();
    expect(resetDefaultExpression).not.toHaveBeenCalled();
  });
});
