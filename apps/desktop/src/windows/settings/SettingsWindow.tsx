import type {
  ChatPlacementDto,
  ThemePreferenceDto,
  UiPreferencesDto,
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
import { PersonalityPanel } from './PersonalityPanel';
import { RuntimeHealthPanel } from './RuntimeHealthPanel';
import { TechnicalLogPanel } from './TechnicalLogPanel';
import { TtsPanel } from './TtsPanel';
import { UpdatesPanel } from './UpdatesPanel';

type ScreenArea = 'settings' | 'personality' | 'conversation' | 'chat';
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

const SETTINGS_ITEMS: Array<{ id: SettingsArea; label: string }> = [
  { id: 'audio', label: '音声' },
  { id: 'ai', label: 'AI' },
  { id: 'character', label: 'キャラクター' },
  { id: 'data', label: 'データ' },
  { id: 'diagnostics', label: '診断' },
  { id: 'display', label: '表示' },
  { id: 'updates', label: '更新' },
];

function TabList<T extends string>({
  idPrefix,
  label,
  items,
  value,
  onChange,
  className,
  autoFocusValue,
}: {
  idPrefix: string;
  label: string;
  items: Array<{ id: T; label: string }>;
  value: T;
  onChange: (value: T) => void;
  className: string;
  autoFocusValue?: T | undefined;
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
    onChange(items[wanted]!.id);
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[wanted]
      ?.focus();
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
          tabIndex={value === item.id ? 0 : -1}
          autoFocus={autoFocusValue === item.id}
          onClick={() => onChange(item.id)}
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
      return <div className="panel-stack"><DataPanel /><ConversationLogPanel /></div>;
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
  const [settingsArea, setSettingsArea] = useState<SettingsArea>('display');
  const [preferences, setPreferences] = useState<UiPreferencesDto>({
    schema_version: 1,
    theme: 'system',
    chat_placement: 'docked',
  });
  const [error, setError] = useState<string | null>(null);
  const restoreScreenMenuFocus = useRef(false);

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
    const unsubscribe = subscribeEvent<UiPreferencesDto>('ui-preferences-changed', (value) => {
      setPreferences(value);
      applyThemePreference(value.theme);
    });
    const stopNavigation = subscribeEvent<string>('control-center-navigate', (value) => {
      if (value === 'conversation') setScreenArea('chat');
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

  const changeScreen = (next: ScreenArea) => {
    if (next !== 'chat') restoreScreenMenuFocus.current = false;
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

  return (
    <main
      aria-label="Parallel World"
      className="control-center"
      data-ui-style="conversation-first"
      data-screen={screenArea}
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
                onChange={setSettingsArea}
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

      {!showOverlay ? (
        <nav className="screen-navigation" aria-label="画面">
          <TabList
            idPrefix="screen"
            label="画面メニュー"
            items={SCREEN_ITEMS}
            value={screenArea}
            onChange={changeScreen}
            className="screen-tabs"
            autoFocusValue={restoreScreenMenuFocus.current ? 'chat' : undefined}
          />
        </nav>
      ) : null}
    </main>
  );
}
