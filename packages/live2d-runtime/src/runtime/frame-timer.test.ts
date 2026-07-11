import { describe, expect, it } from 'vitest';
import { FrameTimer } from './frame-timer';

describe('FrameTimer', () => {
  it('reports elapsed seconds between ticks', () => {
    const timer = new FrameTimer(1000);
    expect(timer.tick(1016)).toBeCloseTo(0.016);
    expect(timer.tick(1048)).toBeCloseTo(0.032);
  });

  it('clamps long pauses so animations never jump', () => {
    const timer = new FrameTimer(0);
    expect(timer.tick(5000)).toBeLessThanOrEqual(0.1);
  });

  it('never returns negative time', () => {
    const timer = new FrameTimer(1000);
    expect(timer.tick(900)).toBe(0);
  });
});
