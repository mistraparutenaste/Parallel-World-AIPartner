import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import type { CharacterPresentationSettingsDto } from '@parallel-world/contracts';
import { tauriCharacterPresentationTransport, setCharacterPresentation } from '../../features/character/tauriCharacterPresentation';
import { ActionButton } from '../../shared/components/ActionButton';
import { Icon } from '../../shared/components/Icons';
import { WindowFrame } from '../../shared/components/WindowFrame';
import '../../shared/styles/global.css';

const navigation = [
  ['マイク', 'microphone'],
  ['音声認識', 'diagnostic'],
  ['LLM', 'model'],
  ['音声合成', 'speaker'],
  ['キャラクター', 'user'],
  ['データ', 'database'],
  ['診断', 'diagnostic'],
] as const;

const initialCharacterPresentation: CharacterPresentationSettingsDto = { schema_version: 1, revision: 0, model_id: 'mark', expression_id: '', motion_group: 'Idle', motion_index: 0, click_through: false };
const defaultLoadCharacterPresentation = async (): Promise<CharacterPresentationSettingsDto> => isTauri()
  ? await tauriCharacterPresentationTransport.get() as CharacterPresentationSettingsDto
  : initialCharacterPresentation;
const defaultSaveCharacterPresentation = setCharacterPresentation;
const expressions = ['Angry', 'Blushing', 'f01', 'f02', 'Normal', 'Sad', 'Smile', 'Surprised'];
const motions = {
  mark: [['Idle', 0], ['Idle', 1], ['Idle', 2], ['Idle', 3], ['Idle', 4], ['Idle', 5]],
  'epsilon-free': [['Idle', 0], ['FlickUp', 0], ['FlickUp', 1], ['Flick', 0], ['Flick', 1], ['Tap', 0], ['Tap', 1], ['Tap', 2], ['Tap', 3], ['Flick3', 0], ['Flick3', 1], ['FlickDown', 0], ['FlickDown', 1], ['Shake', 0], ['Shake', 1]],
} as const;

export interface SettingsWindowProps {
  loadCharacterPresentation?: () => Promise<CharacterPresentationSettingsDto>;
  saveCharacterPresentation?: (value: CharacterPresentationSettingsDto) => Promise<CharacterPresentationSettingsDto>;
}

export function SettingsWindow({
  loadCharacterPresentation = defaultLoadCharacterPresentation,
  saveCharacterPresentation = defaultSaveCharacterPresentation,
}: SettingsWindowProps = {}) {
  const [selected, setSelected] = useState('マイク');
  const [character, setCharacter] = useState(initialCharacterPresentation);
  const [savedCharacter, setSavedCharacter] = useState(initialCharacterPresentation);

  useEffect(() => {
    if (selected !== 'キャラクター') return;
    let active = true;
    void loadCharacterPresentation().then(value => { if (active && value.schema_version === 1) { setCharacter(value); setSavedCharacter(value); } });
    return () => { active = false; };
  }, [selected, loadCharacterPresentation]);

  function handleCancel() {
    if (selected === 'キャラクター') setCharacter(savedCharacter);
    else setSelected('マイク');
  }

  async function handleCharacterApply() {
    const saved = await saveCharacterPresentation(character);
    setCharacter(saved); setSavedCharacter(saved);
  }

  return (
    <WindowFrame title="設定">
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="設定">
          {navigation.map(([label, icon]) => (
            <button key={label} type="button" className={selected === label ? 'is-selected' : ''} aria-current={selected === label ? 'page' : undefined} onClick={() => setSelected(label)}>
              <Icon name={icon} />
              {label}
            </button>
          ))}
        </nav>
        <section className="settings-panel" aria-labelledby="settings-heading">
          <h1 id="settings-heading">設定</h1>
          {selected === 'マイク' ? (
            <>
              <div className="settings-field">
                <label htmlFor="microphone-device">マイクデバイス</label>
                <select id="microphone-device" defaultValue="realtek">
                  <option value="realtek">マイク (Realtek(R) Audio)</option>
                </select>
              </div>
              <div className="settings-field">
                <span className="control-label">入力レベル</span>
                <div className="input-level" role="img" aria-label="入力レベル">
                  {Array.from({ length: 32 }, (_, index) => <i key={index} className={index < 16 ? 'is-active' : ''} />)}
                </div>
              </div>
              <div className="settings-field settings-test">
                <span className="control-label">テスト</span>
                <ActionButton type="button"><Icon name="microphone" />テストを開始</ActionButton>
              </div>
              <footer className="settings-actions">
                <ActionButton type="button" variant="primary" disabled>適用</ActionButton>
                <ActionButton type="button" onClick={handleCancel}>キャンセル</ActionButton>
              </footer>
            </>
          ) : selected === 'キャラクター' ? (
            <>
              <div className="settings-field">
                <label htmlFor="character-model">モデル</label>
                <select id="character-model" value={character.model_id} onChange={event => {
                  const model_id = event.target.value;
                  setCharacter(current => ({ ...current, model_id, expression_id: model_id === 'mark' ? '' : 'Normal', motion_group: 'Idle', motion_index: 0 }));
                }}>
                  <option value="mark">Mark</option><option value="epsilon-free">Epsilon Free</option>
                </select>
              </div>
              <div className="settings-field">
                <label htmlFor="character-expression">表情</label>
                <select id="character-expression" value={character.expression_id} disabled={character.model_id === 'mark'} onChange={event => setCharacter(current => ({ ...current, expression_id: event.target.value }))}>
                  {character.model_id === 'mark' ? <option value="">表情なし</option> : expressions.map(expression => <option key={expression} value={expression}>{expression}</option>)}
                </select>
              </div>
              <div className="settings-field">
                <label htmlFor="character-motion">モーション</label>
                <select id="character-motion" value={`${character.motion_group}:${character.motion_index}`} onChange={event => {
                  const [motion_group, index] = event.target.value.split(':');
                  setCharacter(current => ({ ...current, motion_group: motion_group!, motion_index: Number(index) }));
                }}>
                  {motions[character.model_id as keyof typeof motions].map(([group, index]) => <option key={`${group}:${index}`} value={`${group}:${index}`}>{group} {index + 1}</option>)}
                </select>
              </div>
              <label className="settings-toggle"><input type="checkbox" checked={character.click_through} onChange={event => setCharacter(current => ({ ...current, click_through: event.target.checked }))} />クリック透過</label>
              <footer className="settings-actions">
                <ActionButton type="button" variant="primary" disabled={JSON.stringify(character) === JSON.stringify(savedCharacter)} onClick={() => void handleCharacterApply()}>適用</ActionButton>
                <ActionButton type="button" onClick={handleCancel}>キャンセル</ActionButton>
              </footer>
            </>
          ) : null}
        </section>
      </div>
    </WindowFrame>
  );
}
