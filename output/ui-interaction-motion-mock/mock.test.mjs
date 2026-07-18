import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const directory = new URL('.', import.meta.url);

test('motion preview keeps its review timings when reduced motion is enabled', async () => {
  const [html, css, javascript] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
    readFile(new URL('mock.js', directory), 'utf8'),
  ]);

  assert.match(html, /data-motion="preview"/);
  assert.match(css, /:root:not\(\[data-motion="preview"\]\)/);
  assert.match(javascript, /root\.dataset\.motion === 'preview'/);
});

test('light theme is the review baseline and has a blue-to-purple reflection', async () => {
  const [html, css] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
  ]);

  assert.match(html, /data-theme="light"/);
  assert.match(html, /<option value="light" selected>/);
  assert.match(css, /--reflection-blue:/);
  assert.match(css, /--reflection-purple:/);
  assert.match(css, /var\(--reflection-blue\)[\s\S]*var\(--reflection-purple\)/);
  assert.match(css, /:root\[data-theme="dark"\]/);
});

test('settings entrance starts immediately and runs beside click confirmation', async () => {
  const [css, javascript] = await Promise.all([
    readFile(new URL('styles.css', directory), 'utf8'),
    readFile(new URL('mock.js', directory), 'utf8'),
  ]);
  const functionStart = javascript.indexOf('function confirmMainSelection');
  const functionEnd = javascript.indexOf('\nfunction closeFocusedView', functionStart);
  const confirmationFunction = javascript.slice(functionStart, functionEnd);

  assert.ok(functionStart >= 0 && functionEnd > functionStart);
  assert.ok(
    confirmationFunction.indexOf('startSettingsEntrance()') < confirmationFunction.indexOf('scheduleTransition'),
    'Settings entrance must start before the 500ms confirmation timeout',
  );
  assert.match(confirmationFunction, /chatView\.classList\.add\('is-transitioning'\)/);
  assert.match(css, /\.mock-view--chat\.is-transitioning/);
});

test('focused settings screen is close-only without the legacy bottom navigation', async () => {
  const [html, css] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
  ]);

  assert.match(html, /aria-label="設定を閉じる"/);
  assert.match(html, /role="dialog" aria-modal="true" aria-label="設定画面"[^>]* inert/);
  assert.doesNotMatch(html, /settings-home|settings-chat|composer--muted/);
  assert.match(css, /\.diamond:hover \.diamond__shape::after/);
  assert.doesNotMatch(css, /settings-home|settings-chat|is-static/);
});

test('all seven settings categories use one equal horizontal spacing rule', async () => {
  const [html, css] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
  ]);
  const columns = [...html.matchAll(/data-category="[^"]+" style="--order: \d; --column: (\d)"/g)].map(
    (match) => Number(match[1]),
  );

  assert.deepEqual(columns, [0, 1, 2, 3, 4, 5, 6]);
  assert.match(css, /--category-size: 22\.5%;/);
  assert.match(css, /--category-step: 12\.9166667%;/);
  assert.match(css, /width: min\(830px, 56vw\);/);
  assert.match(css, /aspect-ratio: 830 \/ 285;/);
  assert.match(css, /--category-step:/);
  assert.match(css, /left: calc\(var\(--column\) \* var\(--category-step\)\)/);
});

test('glass reflection uses a clean highlight between bright blue and lavender', async () => {
  const css = await readFile(new URL('styles.css', directory), 'utf8');

  assert.match(css, /--reflection-highlight:/);
  assert.match(
    css,
    /var\(--reflection-blue\)[\s\S]*var\(--reflection-highlight\)[\s\S]*var\(--reflection-purple\)/,
  );
});

test('main diamond cluster keeps its heart proportions at every responsive size', async () => {
  const css = await readFile(new URL('styles.css', directory), 'utf8');

  assert.match(
    css,
    /\.main-menu \{[\s\S]*?--main-diamond-size: clamp\(78px, 9\.7vw, 144px\);[\s\S]*?width: calc\(var\(--main-diamond-size\) \* 2\);[\s\S]*?height: calc\(var\(--main-diamond-size\) \* 1\.5\);[\s\S]*?\}/,
  );
  assert.match(css, /@media \(max-width: 760px\) \{[\s\S]*?\.main-menu \{[\s\S]*?--main-diamond-size: 82px;/);
  assert.doesNotMatch(css, /width: clamp\(220px, 20vw, 300px\)/);
  assert.doesNotMatch(css, /height: clamp\(165px, 18\.5vw, 216px\)/);
  assert.match(css, /\.main-menu \.diamond--settings \{[\s\S]*?bottom: 0;[\s\S]*?left: 25%;[\s\S]*?\}/);
  assert.match(css, /\.main-menu \.diamond--conversation \{[\s\S]*?top: 0;[\s\S]*?left: 0;[\s\S]*?\}/);
});

test('focused views isolate background focus and restore the chat diamond', async () => {
  const javascript = await readFile(new URL('mock.js', directory), 'utf8');

  assert.match(javascript, /closeButton\.focus\(\)/);
  assert.match(javascript, /personalityCloseButton\.focus\(\)/);
  assert.match(javascript, /conversationCloseButton\.focus\(\)/);
  assert.match(javascript, /chatView\.inert = !showChat/);
  assert.match(javascript, /chatButton\.focus\(\)/);
});

test('focused views can close immediately while their entrance transition is running', async () => {
  const javascript = await readFile(new URL('mock.js', directory), 'utf8');
  const closeStart = javascript.indexOf('function closeFocusedView');
  const closeEnd = javascript.indexOf('\n}\n\nfor (const button', closeStart);
  const closeFunction = javascript.slice(closeStart, closeEnd);

  assert.match(closeFunction, /cancelTransitionTimers\(\)/);
  assert.match(closeFunction, /transitionLocked = false/);
  assert.doesNotMatch(closeFunction, /if \(transitionLocked\) return/);
  assert.match(javascript, /if \(event\.key === 'Escape'\)/);
});

test('personality prototype expands three diamond outlines from the clicked control', async () => {
  const [html, css, javascript] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
    readFile(new URL('mock.js', directory), 'utf8'),
  ]);
  const rings = [...html.matchAll(/data-personality-ring/g)];

  assert.equal(rings.length, 3);
  assert.match(html, /data-view="personality"/);
  assert.match(css, /@keyframes personality-ring-expand/);
  assert.match(css, /@keyframes personality-view-in/);
  assert.match(javascript, /function startPersonalityEntrance\(button\)/);
  assert.match(javascript, /button\.getBoundingClientRect\(\)/);
  assert.match(
    javascript,
    /button\.dataset\.mainTarget === 'personality'[\s\S]*?startPersonalityEntrance\(button\)/,
  );
});

test('conversation prototype expands three circular outlines from the clicked control', async () => {
  const [html, css, javascript] = await Promise.all([
    readFile(new URL('index.html', directory), 'utf8'),
    readFile(new URL('styles.css', directory), 'utf8'),
    readFile(new URL('mock.js', directory), 'utf8'),
  ]);
  const rings = [...html.matchAll(/data-conversation-ring/g)];

  assert.equal(rings.length, 3);
  assert.doesNotMatch(html, /data-conversation-trail/);
  assert.doesNotMatch(html, /conversation-transition__node/);
  assert.match(html, /data-view="conversation"/);
  assert.match(css, /@keyframes conversation-ring-expand/);
  assert.doesNotMatch(css, /@keyframes conversation-trail-sweep/);
  assert.doesNotMatch(css, /@keyframes conversation-node-pulse/);
  assert.match(css, /@keyframes conversation-view-in/);
  assert.match(javascript, /function startConversationEntrance\(button\)/);
  assert.match(javascript, /button\.getBoundingClientRect\(\)/);
  assert.match(
    javascript,
    /button\.dataset\.mainTarget === 'conversation'[\s\S]*?startConversationEntrance\(button\)/,
  );
});
