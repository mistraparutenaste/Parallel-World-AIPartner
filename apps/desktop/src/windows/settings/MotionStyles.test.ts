import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const stylesheet = resolve(process.cwd(), 'src/shared/styles/global.css');
const settingsWindow = resolve(process.cwd(), 'src/windows/settings/SettingsWindow.tsx');

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

  it('hides functional conversation content while rebuilding translucency inside overlays', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toMatch(/\.conversation-layer\[aria-hidden='true'\] \{[\s\S]*?opacity: 0;/);
    expect(css).toMatch(/\.screen-overlay \{[\s\S]*?background: var\(--page\);/);
    expect(css).toMatch(/\.screen-overlay--settings \{[\s\S]*?background: var\(--page\);/);
    expect(css).toMatch(/\.screen-overlay::after \{[\s\S]*?linear-gradient/);
    expect(css).toMatch(/@keyframes settings-gradient-in/);
    expect(css).not.toMatch(/animation: settings-category-in/);
  });

  it('defines the approved click and screen transition animations', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toMatch(/\[data-confirming='true'\]/);
    expect(css).toMatch(/@keyframes main-diamond-confirm/);
    expect(css).toMatch(/@keyframes screen-ripple-expand/);
    expect(css).toMatch(/@keyframes screen-view-in/);
    expect(css).toMatch(
      /@keyframes screen-ripple-expand \{[\s\S]*?transform:[\s\S]*?scale\([\s\S]*?\}/,
    );
    expect(css).not.toMatch(
      /@keyframes screen-ripple-expand \{[\s\S]*?(?:width|height): 160vmax/,
    );
  });

  it('matches the approved v9 reflection timing, colors, and centered axis', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toContain('--motion-reflection-blue: #20dcff;');
    expect(css).toContain('--motion-reflection-highlight: #69b9ff;');
    expect(css).toContain('--motion-reflection-purple: #c788ff;');
    expect(css).toMatch(/\.category-crown button:hover::after \{[\s\S]*?190ms/);
    expect(css).toMatch(/\.screen-tabs button:hover::after \{[\s\S]*?320ms/);
    expect(css).toMatch(
      /\.category-crown button::after,[\s\S]*?\.screen-tabs button::after \{[\s\S]*?top: 50%;[\s\S]*?left: 50%;[\s\S]*?transform-origin: center;/,
    );
    expect(css).toMatch(
      /@keyframes glass-reflection \{[\s\S]*?translate\(-50%, -50%\) translateX\(220%\)[\s\S]*?translate\(-50%, -50%\) translateX\(-220%\)/,
    );
  });

  it('moves the reflection band fully across each diamond before fading out', async () => {
    const css = await readFile(stylesheet, 'utf8');
    const bandWidthMatch = css.match(
      /\.category-crown button::after,[\s\S]*?\.screen-tabs button::after \{[\s\S]*?width: (\d+)%;/,
    );
    const travelMatch = css.match(
      /@keyframes glass-reflection \{[\s\S]*?translateX\((\d+)%\)[\s\S]*?translateX\(-(\d+)%\)/,
    );

    expect(bandWidthMatch).not.toBeNull();
    expect(travelMatch).not.toBeNull();

    const bandWidth = Number(bandWidthMatch![1]) / 100;
    const startTravel = Number(travelMatch![1]) / 100 * bandWidth;
    const endTravel = Number(travelMatch![2]) / 100 * bandWidth;
    const startLeftEdge = 0.5 - bandWidth / 2 + startTravel;
    const endRightEdge = 0.5 + bandWidth / 2 - endTravel;

    expect(startLeftEdge).toBeGreaterThanOrEqual(1);
    expect(endRightEdge).toBeLessThanOrEqual(0);
    expect(css).toMatch(
      /@keyframes glass-reflection \{[\s\S]*?12% \{[\s\S]*?opacity: 1;[\s\S]*?translateX\(\d+%\)[\s\S]*?88% \{[\s\S]*?translateX\(-\d+%\)/,
    );
  });

  it('matches the approved v9 diamond spacing and heart order', async () => {
    const css = await readFile(stylesheet, 'utf8');

    expect(css).toContain('--category-gap: 29px;');
    expect(css).toMatch(
      /\.screen-tabs button\[data-tab-id='settings'\] \{[\s\S]*?top: var\(--screen-half-span\);[\s\S]*?left: var\(--screen-half-span\);/,
    );
    expect(css).toMatch(
      /\.screen-tabs button\[data-tab-id='personality'\] \{[\s\S]*?top: 0;[\s\S]*?left: var\(--screen-span\);/,
    );
    expect(css).toMatch(
      /\.screen-tabs button\[data-tab-id='memory'\] \{[\s\S]*?top: 0;[\s\S]*?left: 0;/,
    );
  });

  it('keeps cinematic motion, speeds up repeat visits, and honors reduced motion', async () => {
    const [css, component] = await Promise.all([
      readFile(stylesheet, 'utf8'),
      readFile(settingsWindow, 'utf8'),
    ]);

    expect(component).toContain("matchMedia('(prefers-reduced-motion: reduce)')");
    expect(component).toContain("'quick'");
    expect(css).toMatch(/\[data-transition-speed='quick'\]/);
    expect(css).toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{[\s\S]*?\[data-transition-speed='reduced'\]/,
    );
    expect(css).toMatch(/@keyframes screen-content-fade/);
  });
});
