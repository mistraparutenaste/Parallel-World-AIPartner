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
      /@keyframes glass-reflection \{[\s\S]*?translate\(-50%, -50%\) translateX\(180%\)[\s\S]*?translate\(-50%, -50%\) translateX\(-180%\)/,
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
      /\.screen-tabs button\[data-tab-id='conversation'\] \{[\s\S]*?top: 0;[\s\S]*?left: 0;/,
    );
  });

  it('does not silently disable the approved UI motion from the OS preference', async () => {
    const [css, component] = await Promise.all([
      readFile(stylesheet, 'utf8'),
      readFile(settingsWindow, 'utf8'),
    ]);

    expect(component).not.toContain("matchMedia('(prefers-reduced-motion: reduce)')");
    expect(css).not.toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{[\s\S]*?\.control-center\[data-ui-style='conversation-first'\] \*/,
    );
  });
});
