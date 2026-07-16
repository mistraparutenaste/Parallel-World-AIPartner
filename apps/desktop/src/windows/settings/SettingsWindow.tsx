import type { ThemePreferenceDto, UiPreferencesDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { applyThemePreference } from '../../shared/ui-preferences';
import { ChatWindow } from '../chat/ChatWindow';
import { CharacterPanel } from './CharacterPanel';
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

type MainArea = 'conversation' | 'settings' | 'logs';
type SettingsArea = 'audio' | 'ai' | 'character' | 'data' | 'diagnostics' | 'updates';
type LogArea = 'conversation-log' | 'technical-log';

const MAIN_ITEMS: Array<{ id: MainArea; label: string; icon: string }> = [
  { id: 'conversation', label: '会話', icon: '◌' },
  { id: 'settings', label: '設定', icon: '⌁' },
  { id: 'logs', label: 'ログ', icon: '≡' },
];
const SETTINGS_ITEMS: Array<{ id: SettingsArea; label: string }> = [
  { id: 'audio', label: '音声' },
  { id: 'ai', label: 'AI' },
  { id: 'character', label: 'キャラクター' },
  { id: 'data', label: 'データ' },
  { id: 'diagnostics', label: '診断' },
  { id: 'updates', label: '更新' },
];
const LOG_ITEMS: Array<{ id: LogArea; label: string }> = [
  { id: 'conversation-log', label: '会話ログ' },
  { id: 'technical-log', label: '技術ログ' },
];

function TabList<T extends string>({
  idPrefix,
  label,
  items,
  value,
  onChange,
  orientation = 'horizontal',
  className,
}: {
  idPrefix: string;
  label: string;
  items: Array<{ id: T; label: string; icon?: string }>;
  value: T;
  onChange: (value: T) => void;
  orientation?: 'horizontal' | 'vertical';
  className?: string;
}) {
  const move = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    const previous = orientation === 'vertical' ? 'ArrowUp' : 'ArrowLeft';
    const next = orientation === 'vertical' ? 'ArrowDown' : 'ArrowRight';
    let wanted = index;
    if (event.key === previous) wanted = (index - 1 + items.length) % items.length;
    else if (event.key === next) wanted = (index + 1) % items.length;
    else if (event.key === 'Home') wanted = 0;
    else if (event.key === 'End') wanted = items.length - 1;
    else return;
    event.preventDefault();
    onChange(items[wanted]!.id);
    event.currentTarget.parentElement
      ?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[wanted]
      ?.focus();
  };
  return (
    <div className={className} role="tablist" aria-label={label} aria-orientation={orientation}>
      {items.map((item, index) => (
        <button
          key={item.id}
          id={`${idPrefix}-${item.id}-tab`}
          type="button"
          role="tab"
          aria-controls={`${idPrefix}-${item.id}-panel`}
          aria-selected={value === item.id}
          tabIndex={value === item.id ? 0 : -1}
          onClick={() => onChange(item.id)}
          onKeyDown={(event) => move(event, index)}
        >
          {item.icon ? <span aria-hidden="true">{item.icon}</span> : null}
          <span>{item.label}</span>
        </button>
      ))}
    </div>
  );
}

function SettingsContent({ active }: { active: SettingsArea }) {
  switch (active) {
    case 'audio': return <div className="panel-stack"><MicrophonePanel /><TtsPanel /></div>;
    case 'ai': return <div className="panel-stack"><LlmPanel /></div>;
    case 'character': return <div className="panel-stack"><CharacterPanel /><PersonalityPanel /></div>;
    case 'data': return <div className="panel-stack"><DataPanel /></div>;
    case 'diagnostics': return <div className="panel-stack"><RuntimeHealthPanel /><DiagnosticsPanel /></div>;
    case 'updates': return <div className="panel-stack"><UpdatesPanel /></div>;
  }
}

export function SettingsWindow() {
  const [mainArea, setMainArea] = useState<MainArea>('conversation');
  const [settingsArea, setSettingsArea] = useState<SettingsArea>('audio');
  const [logArea, setLogArea] = useState<LogArea>('conversation-log');
  const [preferences, setPreferences] = useState<UiPreferencesDto>({
    schema_version: 1,
    theme: 'system',
    chat_placement: 'docked',
  });
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    invoke<UiPreferencesDto>('get_ui_preferences')
      .then((value) => {
        if (mounted) {
          setPreferences(value);
          applyThemePreference(value.theme);
        }
      })
      .catch((problem) => mounted && setError(String(problem)));
    const unsubscribe = subscribeEvent<UiPreferencesDto>('ui-preferences-changed', (value) => {
      setPreferences(value);
      applyThemePreference(value.theme);
    });
    const stopNavigation = subscribeEvent<string>('control-center-navigate', (value) => {
      if (value === 'conversation') setMainArea('conversation');
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
    } catch (problem) {
      setPreferences(previous);
      applyThemePreference(previous.theme);
      setError(String(problem));
    }
  };

  const setPlacement = async () => {
    try {
      const value = await invoke<UiPreferencesDto>('set_chat_placement', {
        placement: 'docked',
      });
      setPreferences(value);
      setError(null);
    } catch (problem) {
      setError(String(problem));
    }
  };

  return (
    <main
      aria-label="管理画面"
      className="control-center"
      data-ui-style="geometric-game"
    >
      <aside className="control-sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true" /><strong>Parallel World</strong></div>
        <TabList idPrefix="control-main" label="管理メニュー" items={MAIN_ITEMS} value={mainArea} onChange={setMainArea} orientation="vertical" className="main-tabs" />
      </aside>
      <section className="control-workspace">
        <header className="control-header">
          <div><p className="eyebrow">Control center</p><h1>{MAIN_ITEMS.find((item) => item.id === mainArea)?.label}</h1></div>
          <label className="theme-control">
            <span>テーマ</span>
            <select value={preferences.theme} onChange={(event) => void setTheme(event.target.value as ThemePreferenceDto)}>
              <option value="system">システム</option>
              <option value="light">ライト</option>
              <option value="dark">ダーク</option>
            </select>
          </label>
        </header>
        {error ? <p className="global-error" role="alert">{error}</p> : null}
        <div
          id={`control-main-${mainArea}-panel`}
          className="control-content"
          role="tabpanel"
          aria-labelledby={`control-main-${mainArea}-tab`}
        >
          {mainArea === 'conversation' ? (
            preferences.chat_placement === 'popped' ? (
              <section className="empty-state detached-chat"><h2>会話は別ウィンドウで開いています</h2><button type="button" onClick={() => void setPlacement()}>再格納</button></section>
            ) : <ChatWindow placementControl="popout" />
          ) : null}
          {mainArea === 'settings' ? (
            <>
              <TabList idPrefix="control-settings" label="設定カテゴリ" items={SETTINGS_ITEMS} value={settingsArea} onChange={setSettingsArea} className="sub-tabs" />
              <div id={`control-settings-${settingsArea}-panel`} role="tabpanel" aria-labelledby={`control-settings-${settingsArea}-tab`}>
                <SettingsContent active={settingsArea} />
              </div>
            </>
          ) : null}
          {mainArea === 'logs' ? (
            <>
              <TabList idPrefix="control-logs" label="ログ種別" items={LOG_ITEMS} value={logArea} onChange={setLogArea} className="sub-tabs" />
              <div id={`control-logs-${logArea}-panel`} role="tabpanel" aria-labelledby={`control-logs-${logArea}-tab`}>
                {logArea === 'conversation-log' ? <ConversationLogPanel /> : <TechnicalLogPanel />}
              </div>
            </>
          ) : null}
        </div>
      </section>
    </main>
  );
}
