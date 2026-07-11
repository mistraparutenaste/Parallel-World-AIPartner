import { describe, expect, it } from 'vitest';
import { modelBaseUrl, resolveResourceUrl } from './model-url';

describe('modelBaseUrl', () => {
  it('returns the directory of a model3.json url', () => {
    expect(
      modelBaseUrl('http://asset.localhost/characters/epsilon/e.model3.json'),
    ).toBe('http://asset.localhost/characters/epsilon/');
  });

  it('keeps query-free relative paths intact', () => {
    expect(modelBaseUrl('/live2d/models/eps/e.model3.json')).toBe(
      '/live2d/models/eps/',
    );
  });
});

describe('resolveResourceUrl', () => {
  it('joins the base directory with a relative resource path', () => {
    expect(
      resolveResourceUrl('/models/eps/', 'motion/idle_01.motion3.json'),
    ).toBe('/models/eps/motion/idle_01.motion3.json');
  });

  it('encodes each path segment so spaces survive asset urls', () => {
    expect(resolveResourceUrl('/models/eps/', 'tex 01/texture.png')).toBe(
      '/models/eps/tex%2001/texture.png',
    );
  });
});
