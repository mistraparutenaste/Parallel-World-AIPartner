import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const stylesheet = resolve(process.cwd(), 'src/shared/styles/global.css');

describe('conversation-first motion styles', () => {
  it('keeps pointer hit testing on the same rotated surface that is rendered', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toMatch(
      /\.category-crown button \{[\s\S]*?transform: translate\(-50%, -50%\) rotate\(45deg\);/,
    );
    expect(css).toMatch(/\.screen-tabs button \{[\s\S]*?transform: rotate\(45deg\);/);
    expect(css).toMatch(/\.category-crown button:hover::after/);
    expect(css).toMatch(/\.screen-tabs button:hover::after/);
  });

  it('keeps settings transparent over the existing conversation background', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toMatch(/\.screen-overlay--settings \{[\s\S]*?background: transparent;/);
    expect(css).toMatch(/\[data-screen='settings'\][\s\S]*?\.conversation-layer/);
    expect(css).toMatch(/@keyframes settings-gradient-in/);
    expect(css).toMatch(/@keyframes settings-category-in/);
  });

  it('defines the approved click and screen transition animations', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toMatch(/\[data-confirming='true'\]/);
    expect(css).toMatch(/@keyframes main-diamond-confirm/);
    expect(css).toMatch(/@keyframes personality-ring-expand/);
    expect(css).toMatch(/@keyframes personality-view-in/);
    expect(css).toMatch(/@keyframes conversation-ring-expand/);
    expect(css).toMatch(/@keyframes conversation-view-in/);
  });
});
