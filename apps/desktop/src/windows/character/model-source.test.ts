import { describe, expect, it } from 'vitest';
import { createModelSource } from './model-source';

const convert = (path: string) => `asset://${encodeURIComponent(path)}`;

describe('createModelSource', () => {
  it('resolves relative resources against a windows model path', () => {
    const source = createModelSource(
      'C:\\data\\characters\\eps\\Epsilon.model3.json',
      convert,
    );
    expect(source.modelUrl).toBe(
      convert('C:\\data\\characters\\eps\\Epsilon.model3.json'),
    );
    expect(source.resolveResource('motion/idle_01.motion3.json')).toBe(
      convert('C:\\data\\characters\\eps\\motion\\idle_01.motion3.json'),
    );
  });

  it('resolves relative resources against a posix model path', () => {
    const source = createModelSource(
      '/home/user/characters/eps/Epsilon.model3.json',
      convert,
    );
    expect(source.resolveResource('expressions/Smile.exp3.json')).toBe(
      convert('/home/user/characters/eps/expressions/Smile.exp3.json'),
    );
  });
});
