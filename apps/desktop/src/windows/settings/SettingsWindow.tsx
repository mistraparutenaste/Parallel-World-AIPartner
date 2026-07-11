import { useState } from 'react';
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

export function SettingsWindow() {
  const [selected, setSelected] = useState('マイク');

  function handleCancel() {
    setSelected('マイク');
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
          ) : null}
        </section>
      </div>
    </WindowFrame>
  );
}
