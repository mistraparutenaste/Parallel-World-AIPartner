import { describe, expect, it } from 'vitest';
import { getLive2DCharacterManifest } from './Live2DCharacterDev';

describe('Live2D development catalog', () => {
  it('provides real Mark and Epsilon model catalogs', () => {
    expect(getLive2DCharacterManifest('mark').model3).toBe('Mark.model3.json');
    expect(getLive2DCharacterManifest('mark').motions.Idle).toEqual([0, 1, 2, 3, 4, 5]);
    const epsilon = getLive2DCharacterManifest('epsilon-free');
    expect(epsilon.model3).toBe('Epsilon_free.model3.json');
    expect(epsilon.expressions.Smile).toBe('expressions/Smile.exp3.json');
    expect(epsilon.motions.Tap).toEqual([0, 1, 2, 3]);
  });
});
