import { describe, expect, it, vi } from 'vitest';
import { SpeechHopController } from './speech-hop';

interface FakeAnimation {
  cancel: ReturnType<typeof vi.fn>;
}

function harness(reducedMotion = false, baseTransform = '') {
  const animations: FakeAnimation[] = [];
  const animate = vi.fn(() => {
    const animation = { cancel: vi.fn() };
    animations.push(animation);
    return animation as unknown as Animation;
  });
  const element = document.createElement('canvas');
  element.style.transform = baseTransform;
  Object.defineProperty(element, 'animate', { configurable: true, value: animate });
  const matchMedia = vi.fn(() => ({ matches: reducedMotion } as MediaQueryList));
  return { element, animate, animations, matchMedia };
}

describe('SpeechHopController', () => {
  it('animates a new turn with the specified hop keyframes and timing', () => {
    const h = harness();
    const controller = new SpeechHopController(h.element, h.matchMedia);

    expect(controller.react(1)).toBe(true);
    expect(h.animate).toHaveBeenCalledWith(
      [
        { transform: 'translateY(0)' },
        { transform: 'translateY(-12px)' },
        { transform: 'translateY(0)' },
      ],
      { duration: 300, easing: 'cubic-bezier(.2,.8,.3,1)' },
    );
  });

  it('deduplicates the same turn and cancels the previous animation for a new turn', () => {
    const h = harness();
    const controller = new SpeechHopController(h.element, h.matchMedia);

    expect(controller.react(4)).toBe(true);
    expect(controller.react(4)).toBe(false);
    expect(h.animate).toHaveBeenCalledTimes(1);

    expect(controller.react(5)).toBe(true);
    expect(h.animations[0]?.cancel).toHaveBeenCalledOnce();
    expect(h.animate).toHaveBeenCalledTimes(2);
  });

  it('records a turn but does not animate when reduced motion is requested', () => {
    const h = harness(true);
    const controller = new SpeechHopController(h.element, h.matchMedia);

    expect(controller.react(8)).toBe(true);
    expect(controller.react(8)).toBe(false);
    expect(h.animate).not.toHaveBeenCalled();
    expect(h.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
  });

  it('cancel stops the animation and restores the base transform', () => {
    const h = harness(false, 'scale(0.75)');
    const controller = new SpeechHopController(h.element, h.matchMedia);
    controller.react(2);
    h.element.style.transform = 'translateY(-12px)';

    controller.cancel();

    expect(h.animations[0]?.cancel).toHaveBeenCalledOnce();
    expect(h.element.style.transform).toBe('scale(0.75)');
  });
});
