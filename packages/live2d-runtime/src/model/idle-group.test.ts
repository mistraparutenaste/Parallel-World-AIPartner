import { describe, expect, it } from 'vitest';
import { resolveIdleGroup } from './idle-group';

describe('resolveIdleGroup', () => {
  it('prefers the canonical Idle group', () => {
    expect(resolveIdleGroup(['Tap', 'Idle', 'Shake'])).toBe('Idle');
  });

  it('matches idle case-insensitively', () => {
    expect(resolveIdleGroup(['tap', 'idle'])).toBe('idle');
  });

  it('falls back to the first group when no idle group exists', () => {
    expect(resolveIdleGroup(['Greeting', 'Tap'])).toBe('Greeting');
  });

  it('returns null when the model has no motion groups', () => {
    expect(resolveIdleGroup([])).toBeNull();
  });
});
