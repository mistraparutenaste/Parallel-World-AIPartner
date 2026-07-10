# Phase 0 UI Design System

## Source of truth

- Concept: `docs/design/concepts/phase0-three-windows.png`
- Native concept size: 1696 × 929
- Theme: quiet midnight desktop, restrained indigo accent, open rails instead of card grids

## Color tokens

```css
:root {
  --pw-bg: #071323;
  --pw-bg-deep: #04101d;
  --pw-surface: rgba(10, 24, 42, 0.94);
  --pw-surface-soft: rgba(18, 35, 61, 0.72);
  --pw-surface-selected: #202f62;
  --pw-border: #31435d;
  --pw-border-strong: #536989;
  --pw-text: #f1f4fb;
  --pw-text-muted: #a7b2c8;
  --pw-text-subtle: #77859f;
  --pw-accent: #7183ff;
  --pw-accent-strong: #6275f4;
  --pw-focus: #8b96ff;
  --pw-danger: #e46f78;
  --pw-success: #68c4a2;
  --pw-shadow: rgba(0, 0, 0, 0.38);
}
```

The background is cool near-black/navy, not white, cream, beige, or warm gray. Accent gradients are prohibited; buttons use solid accent colors.

## Typography

- Family: `Inter`, `Noto Sans JP`, `Yu Gothic UI`, system sans-serif
- Window title: 22px / 1.25 / 500
- Screen heading: 20px / 1.3 / 600
- Empty-state heading: 18px / 1.4 / 500
- Body: 14px / 1.7 / 400
- Control label: 13px / 1.4 / 500
- Compact status: 12px / 1.35 / 500
- Buttons: 14px / 1 / 600

## Spacing and geometry

- Spacing scale: 4, 8, 12, 16, 24, 32, 48px
- Window radius: 8px
- Control radius: 6px
- Border: 1px hairline
- Input/button height: 48px
- Sidebar width: 160px
- Title bar height: 64px
- Focus ring: 2px `--pw-focus`, 2px offset
- No nested card stacks. Use borders, rails, and whitespace.

## Window inventory

### Character Window

- Borderless transparent overlay
- Code-native drag affordance
- Live2D stage fills the window
- Only visible status copy in Phase 0: `準備中`
- No toolbar, settings, badges, or cards

### Chat Window

- Header: `Parallel World`
- Connection state: `待機中`
- Empty-state heading: `会話をはじめましょう`
- Input accessible label and placeholder: `メッセージ`
- Actions: `送信`, `停止`
- Status labels: `STT`, `LLM`, `TTS`, each initially `待機中`
- Conversation area is one open rail, not a grid

### Settings Window

- Heading: `設定`
- Navigation: `マイク`, `音声認識`, `LLM`, `音声合成`, `キャラクター`, `データ`, `診断`
- Phase 0 selected item: `マイク`
- Primary panel labels: `マイクデバイス`, `入力レベル`, `テスト`
- Test action: `テストを開始`
- Bottom actions: `適用`, `キャンセル`

## Icon system

- Rounded outline icons, 1.75px stroke, 20px optical size
- `currentColor`, round caps and joins
- Icons clarify navigation or actions only; no decorative icon rows
- Text labels remain visible; icon-only controls require accessible names

## Interaction rules

- Hover changes border/text contrast without glow.
- Selected navigation uses solid indigo surface and a 3px accent rail.
- Disabled controls reduce opacity but preserve readable contrast.
- Danger actions use `--pw-danger` and require confirmation in later phases.
- Motion duration 120–180ms; disable nonessential motion for `prefers-reduced-motion`.

## Component ownership

- `WindowFrame`: shared title bar and window surface
- `StatusBadge`: dot + status text, semantic tone variants
- `ActionButton`: primary, secondary, danger variants
- `SettingsNavigation`: settings-only rail
- `CharacterWindow`, `ChatWindow`, `SettingsWindow`: composition only
- Entry files mount exactly one window and contain no feature logic

## Allowed first-viewport copy

No additional visible copy may be added above the fold without a recorded design change.

```text
Parallel World
準備中
待機中
会話をはじめましょう
メッセージ
送信
停止
STT
LLM
TTS
設定
マイク
音声認識
音声合成
キャラクター
データ
診断
マイクデバイス
入力レベル
テスト
テストを開始
適用
キャンセル
```

