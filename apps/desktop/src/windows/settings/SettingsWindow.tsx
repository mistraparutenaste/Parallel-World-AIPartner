import { CharacterPanel } from './CharacterPanel';
import { LlmPanel } from './LlmPanel';
import { MicrophonePanel } from './MicrophonePanel';
import { TtsPanel } from './TtsPanel';
import { DataPanel } from './DataPanel';
import { RuntimeHealthPanel } from './RuntimeHealthPanel';
import { DiagnosticsPanel } from './DiagnosticsPanel';

const SETTINGS_SECTIONS = [
  'マイク',
  '音声認識',
  'LLM',
  '音声合成',
  'キャラクター',
  'データ',
  '診断',
] as const;

/**
 * Settings window shell. Each nav item will map to a dedicated
 * settings panel; destructive operations always require confirmation.
 */
export function SettingsWindow() {
  return (
    <main aria-label="設定画面">
      <h1>設定</h1>
      <nav aria-label="設定メニュー">
        <ul>
          {SETTINGS_SECTIONS.map((section) => (
            <li key={section}>{section}</li>
          ))}
        </ul>
      </nav>
      <MicrophonePanel />
      <LlmPanel />
      <TtsPanel />
      <CharacterPanel />
      <DataPanel />
      <RuntimeHealthPanel />
      <DiagnosticsPanel />
    </main>
  );
}
