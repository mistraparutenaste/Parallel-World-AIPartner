import type {
  ChatPlacementDto,
  ThemePreferenceDto,
  UiPreferencesDto,
} from '@parallel-world/contracts';
import {
  CONTROL_CENTER_NAVIGATE_EVENT,
  UI_PREFERENCES_CHANGED_EVENT,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { applyThemePreference } from '../../shared/ui-preferences';
import { ChatWindow } from '../chat/ChatWindow';
import { CharacterPanel } from './CharacterPanel';
import { ConversationSettingsPanel } from './ConversationSettingsPanel';
import { ConversationLogPanel } from './ConversationLogPanel';
import { DataPanel } from './DataPanel';
import { DiagnosticsPanel } from './DiagnosticsPanel';
import { LlmPanel } from './LlmPanel';
import { MicrophonePanel } from './MicrophonePanel';
import { MemoryCenterPanel } from './MemoryCenterPanel';
import { PersonalityPanel } from './PersonalityPanel';
import { RuntimeHealthPanel } from './RuntimeHealthPanel';
import { TechnicalLogPanel } from './TechnicalLogPanel';
import { TtsPanel } from './TtsPanel';
import { UpdatesPanel } from './UpdatesPanel';

type ScreenArea = 'settings' | 'personality' | 'conversation' | 'chat';
type TransitionScreen = Exclude<ScreenArea, 'chat'>;
type SettingsArea =
  | 'audio'
  | 'ai'
  | 'character'
  | 'data'
  | 'diagnostics'
  | 'display'
  | 'updates';

const SCREEN_ITEMS: Array<{ id: ScreenArea; label: string }> = [
  { id: 'settings', label: '設定' },
  { id: 'personality', label: '性格' },
  { id: 'conversation', label: '会話' },
  { id: 'chat', label: 'チャット' },
];

const SETTINGS_ITEMS: Array<{ id: SettingsArea; label: string; enterOrder: number }> = [
  { id: 'audio', label: '音声', enterOrder: 0 },
  { id: 'ai', label: 'AI', enterOrder: 2 },
  { id: 'character', label: 'キャラクター', enterOrder: 4 },
  { id: 'data', label: 'データ', enterOrder: 6 },
  { id: 'diagnostics', label: '診断', enterOrder: 1 },
  { id: 'display', label: '表示', enterOrder: 3 },
  { id: 'updates', label: '更新', enterOrder: 5 },
];

const SCREEN_TRANSITION_DURATION: Record<TransitionScreen, number> = {
  settings: 760,
  personality: 760,
  conversation: 760,
};

const QUICK_SCREEN_TRANSITION_DURATION: Record<TransitionScreen, number> = {
  settings: 420,
  personality: 420,
  conversation: 420,
};

type ScreenTransition = {
  target: TransitionScreen;
  originX: number;
  originY: number;
  originSize: number;
  speed: 'cinematic' | 'quick' | 'reduced';
};

function TabList<T extends string>({
  idPrefix,
  label,
  items,
  value,
  onChange,
  className,
  autoFocusValue,
  confirmingValue,
}: {
  idPrefix: string;
  label: string;
  items: Array<{ id: T; label: string; enterOrder?: number }>;
  value: T;
  onChange: (value: T, source?: HTMLButtonElement) => void;
  className: string;
  autoFocusValue?: T | undefined;
  confirmingValue?: T | undefined;
}) {
  const move = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    let wanted = index;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
      wanted = (index - 1 + items.length) % items.length;
    } else if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
      wanted = (index + 1) % items.length;
    } else if (event.key === 'Home') {
      wanted = 0;
    } else if (event.key === 'End') {
      wanted = items.length - 1;
    } else {
      return;
    }
    event.preventDefault();
    const wantedButton = event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[wanted];
    onChange(items[wanted]!.id, wantedButton);
    wantedButton?.focus();
  };

  return (
    <div className={className} role="tablist" aria-label={label}>
      {items.map((item, index) => (
        <button
          key={item.id}
          id={`${idPrefix}-${item.id}-tab`}
          type="button"
          role="tab"
          aria-label={item.label}
          aria-controls={`${idPrefix}-${item.id}-panel`}
          aria-selected={value === item.id}
          data-tab-id={item.id}
          data-enter-order={item.enterOrder}
          data-confirming={confirmingValue === item.id || undefined}
          style={
            item.enterOrder === undefined
              ? undefined
              : ({ '--enter-delay': `${item.enterOrder * 33}ms` } as React.CSSProperties)
          }
          tabIndex={value === item.id ? 0 : -1}
          autoFocus={autoFocusValue === item.id}
          onClick={(event) => onChange(item.id, event.currentTarget)}
          onKeyDown={(event) => move(event, index)}
        >
          <span aria-hidden="true">
            {item.id === 'character' ? (
              <>
                キャラ
                <br />
                クター
              </>
            ) : item.label}
          </span>
        </button>
      ))}
    </div>
  );
}

function DisplaySettings({
  preferences,
  onThemeChange,
  onPlacementChange,
}: {
  preferences: UiPreferencesDto;
  onThemeChange: (theme: ThemePreferenceDto) => void;
  onPlacementChange: (placement: ChatPlacementDto) => void;
}) {
  return (
    <div className="panel-stack display-settings">
      <section aria-labelledby="display-theme-heading">
        <h2 id="display-theme-heading">テーマ</h2>
        <div className="setting-row">
          <div>
            <strong>画面の明るさ</strong>
            <p>システム設定に合わせるか、ライト／ダークを固定します。</p>
          </div>
          <label>
            <span>テーマ</span>
            <select
              value={preferences.theme}
              onChange={(event) => onThemeChange(event.target.value as ThemePreferenceDto)}
            >
              <option value="system">システム</option>
              <option value="light">ライト</option>
              <option value="dark">ダーク</option>
            </select>
          </label>
        </div>
      </section>

      <section aria-labelledby="display-chat-heading">
        <h2 id="display-chat-heading">チャット表示</h2>
        <div className="setting-row">
          <div>
            <strong>会話を表示する場所</strong>
            <p>この画面に収めるか、独立したウィンドウで表示します。</p>
          </div>
          <label>
            <span>チャット表示</span>
            <select
              value={preferences.chat_placement}
              onChange={(event) => onPlacementChange(event.target.value as ChatPlacementDto)}
            >
              <option value="docked">この画面</option>
              <option value="popped">別ウィンドウ</option>
            </select>
          </label>
        </div>
      </section>
    </div>
  );
}

function SettingsContent({
  active,
  preferences,
  onThemeChange,
  onPlacementChange,
}: {
  active: SettingsArea;
  preferences: UiPreferencesDto;
  onThemeChange: (theme: ThemePreferenceDto) => void;
  onPlacementChange: (placement: ChatPlacementDto) => void;
}) {
  switch (active) {
    case 'audio':
      return <div className="panel-stack"><MicrophonePanel /><TtsPanel /></div>;
    case 'ai':
      return <div className="panel-stack"><LlmPanel /></div>;
    case 'character':
      return <div className="panel-stack"><CharacterPanel /></div>;
    case 'data':
      return <div className="panel-stack"><MemoryCenterPanel /><DataPanel /><ConversationLogPanel /></div>;
    case 'diagnostics':
      return (
        <div className="panel-stack">
          <RuntimeHealthPanel />
          <DiagnosticsPanel />
          <TechnicalLogPanel />
        </div>
      );
    case 'display':
      return (
        <DisplaySettings
          preferences={preferences}
          onThemeChange={onThemeChange}
          onPlacementChange={onPlacementChange}
        />
      );
    case 'updates':
      return <div className="panel-stack"><UpdatesPanel /></div>;
  }
}

export function SettingsWindow() {
  const [screenArea, setScreenArea] = useState<ScreenArea>('chat');
  const [screenTransition, setScreenTransition] = useState<ScreenTransition | null>(null);
  const [settingsArea, setSettingsArea] = useState<SettingsArea>('display');
  const [preferences, setPreferences] = useState<UiPreferencesDto>({
    schema_version: 1,
    theme: 'system',
    chat_placement: 'docked',
  });
  const [error, setError] = useState<string | null>(null);
  const restoreScreenMenuFocus = useRef(false);
  const screenTransitionTimer = useRef<number | null>(null);
  const screenNavigationRef = useRef<HTMLElement | null>(null);
  const visitedScreens = useRef(new Set<TransitionScreen>());

  const cancelScreenTransition = () => {
    if (screenTransitionTimer.current !== null) {
      window.clearTimeout(screenTransitionTimer.current);
      screenTransitionTimer.current = null;
    }
    setScreenTransition(null);
  };

  useEffect(() => () => {
    if (screenTransitionTimer.current !== null) {
      window.clearTimeout(screenTransitionTimer.current);
    }
  }, []);

  useEffect(() => {
    if (screenArea !== 'chat' || !restoreScreenMenuFocus.current) return;
    screenNavigationRef.current
      ?.querySelector<HTMLButtonElement>('[data-tab-id="chat"]')
      ?.focus();
    restoreScreenMenuFocus.current = false;
  }, [screenArea]);

  useEffect(() => {
    let mounted = true;
    invoke<UiPreferencesDto>('get_ui_preferences')
      .then((value) => {
        if (mounted && value) {
          setPreferences(value);
          applyThemePreference(value.theme);
        }
      })
      .catch(() => {
        if (mounted) setError('設定を読み込めません。診断を確認してください。');
      });
    const unsubscribe = subscribeEvent<UiPreferencesDto>(UI_PREFERENCES_CHANGED_EVENT, (value) => {
      setPreferences(value);
      applyThemePreference(value.theme);
    });
    const stopNavigation = subscribeEvent<string>(CONTROL_CENTER_NAVIGATE_EVENT, (value) => {
      if (value === 'conversation') {
        cancelScreenTransition();
        setScreenArea('chat');
      }
    });
    return () => {
      mounted = false;
      unsubscribe();
      stopNavigation();
    };
  }, []);

  const setTheme = async (theme: ThemePreferenceDto) => {
    const previous = preferences;
    const optimistic = { ...preferences, theme };
    setPreferences(optimistic);
    applyThemePreference(theme);
    try {
      const value = await invoke<UiPreferencesDto>('set_theme_preference', { theme });
      if (value) {
        setPreferences(value);
        applyThemePreference(value.theme);
      }
      setError(null);
    } catch {
      setPreferences(previous);
      applyThemePreference(previous.theme);
      setError('表示設定を変更できません。少し待って、もう一度試してください。');
    }
  };

  const setPlacement = async (placement: ChatPlacementDto) => {
    const previous = preferences;
    if (previous.chat_placement !== placement) {
      setPreferences({ ...previous, chat_placement: placement });
    }
    try {
      const value = await invoke<UiPreferencesDto>('set_chat_placement', { placement });
      if (value) setPreferences(value);
      setError(null);
    } catch {
      setPreferences(previous);
      setError('チャット表示を変更できません。少し待って、もう一度試してください。');
    }
  };

  const changeScreen = (next: ScreenArea, source?: HTMLButtonElement) => {
    cancelScreenTransition();
    if (next !== 'chat') restoreScreenMenuFocus.current = false;

    if (next !== 'chat') {
      const reduceMotion = typeof window.matchMedia === 'function'
        && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      const speed = reduceMotion
        ? 'reduced'
        : visitedScreens.current.has(next)
          ? 'quick'
          : 'cinematic';
      visitedScreens.current.add(next);
      const rect = source?.getBoundingClientRect();
      const transformedSize = Math.max(rect?.width ?? 0, rect?.height ?? 0);
      const originSize = next === 'personality'
        ? source?.offsetWidth || transformedSize / Math.SQRT2 || 104
        : transformedSize || 104;
      const transition: ScreenTransition = {
        target: next,
        originX: rect ? rect.left + rect.width / 2 : window.innerWidth / 2,
        originY: rect ? rect.top + rect.height / 2 : window.innerHeight / 2,
        originSize,
        speed,
      };
      setScreenTransition(transition);
      screenTransitionTimer.current = window.setTimeout(() => {
        setScreenTransition(null);
        screenTransitionTimer.current = null;
      }, speed === 'reduced'
        ? 150
        : speed === 'quick'
          ? QUICK_SCREEN_TRANSITION_DURATION[next]
          : SCREEN_TRANSITION_DURATION[next]);
    }

    setScreenArea(next);
    if (next === 'chat' && preferences.chat_placement === 'popped') {
      void setPlacement('popped');
    }
  };

  const closeFocusedScreen = () => {
    restoreScreenMenuFocus.current = true;
    changeScreen('chat');
  };

  const showOverlay = screenArea !== 'chat';
  const chatDocked = preferences.chat_placement === 'docked';
  const activeScreenLabel = SCREEN_ITEMS.find((item) => item.id === screenArea)?.label ?? 'チャット';
  const transitionStyle = screenTransition
    ? ({
        '--screen-origin-x': `${screenTransition.originX}px`,
        '--screen-origin-y': `${screenTransition.originY}px`,
        '--screen-origin-size': `${screenTransition.originSize}px`,
      } as React.CSSProperties)
    : undefined;

  return (
    <main
      aria-label="Parallel World"
      className="control-center"
      data-ui-style="conversation-first"
      data-screen={screenArea}
      data-transition={screenTransition?.target}
      data-transition-speed={screenTransition?.speed}
      style={transitionStyle}
    >
      <section
        id="screen-chat-panel"
        className="conversation-layer"
        role="tabpanel"
        aria-labelledby="screen-chat-tab"
        aria-hidden={showOverlay}
      >
        <aside className="conversation-footprints" aria-label="会話の足跡">
          <ol />
        </aside>
        <div className="conversation-stage">
          {chatDocked ? <ChatWindow inactive={showOverlay} /> : null}
        </div>
      </section>

      {showOverlay ? (
        <section
          id={`screen-${screenArea}-panel`}
          className={`screen-overlay screen-overlay--${screenArea}`}
          role="dialog"
          aria-modal="true"
          aria-label={`${activeScreenLabel}画面`}
          data-transition-stage={screenTransition ? 'unified' : undefined}
          data-motion-shape={
            screenArea === 'settings' || screenArea === 'personality'
              ? 'diamond'
              : screenArea === 'conversation'
                ? 'circle'
                : 'gradient'
          }
        >
          <button
            type="button"
            className="screen-close-button"
            aria-label={`${activeScreenLabel}を閉じる`}
            onClick={closeFocusedScreen}
            autoFocus
          >
            <span aria-hidden="true">×</span>
          </button>

          {error ? <p className="global-error" role="alert">{error}</p> : null}

          {screenArea === 'settings' ? (
            <div className="settings-screen">
              <TabList
                idPrefix="settings-category"
                label="設定カテゴリ"
                items={SETTINGS_ITEMS}
                value={settingsArea}
                onChange={(value) => setSettingsArea(value)}
                className="category-crown"
              />
              <div
                id={`settings-category-${settingsArea}-panel`}
                className="settings-body"
                role="tabpanel"
                aria-labelledby={`settings-category-${settingsArea}-tab`}
              >
                <SettingsContent
                  active={settingsArea}
                  preferences={preferences}
                  onThemeChange={(theme) => void setTheme(theme)}
                  onPlacementChange={(placement) => void setPlacement(placement)}
                />
              </div>
            </div>
          ) : null}

          {screenArea === 'personality' ? (
            <div className="screen-body personality-screen">
              <PersonalityPanel />
            </div>
          ) : null}

          {screenArea === 'conversation' ? (
            <div className="screen-body">
              <ConversationSettingsPanel />
            </div>
          ) : null}
        </section>
      ) : null}

      {screenTransition ? (
        <div
          className={`screen-transition-rings screen-transition-rings--${screenTransition.target}`}
          aria-hidden="true"
        >
          {(['blue', 'highlight', 'purple'] as const).map((tone) => (
            <span
              key={tone}
              className={`screen-transition-ring screen-transition-ring--${tone}`}
              data-transition-ring={
                screenTransition.target === 'conversation' ? 'circle' : 'diamond'
              }
            />
          ))}
        </div>
      ) : null}

      {!showOverlay || screenTransition ? (
        <nav
          ref={screenNavigationRef}
          className="screen-navigation"
          aria-label="画面"
          aria-hidden={showOverlay || undefined}
          inert={showOverlay || undefined}
        >
          <TabList
            idPrefix="screen"
            label="画面メニュー"
            items={SCREEN_ITEMS}
            value={screenArea}
            onChange={changeScreen}
            className="screen-tabs"
            autoFocusValue={restoreScreenMenuFocus.current ? 'chat' : undefined}
            confirmingValue={screenTransition?.target}
          />
        </nav>
      ) : null}
    </main>
  );
}
