import type {
  CharacterManifestDto,
  CharacterSettingsDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';

const IDLE_TIMEOUT_OPTIONS = [
  { value: 'never', label: '戻さない' },
  { value: '10', label: '10秒' },
  { value: '20', label: '20秒' },
  { value: '30', label: '30秒' },
  { value: '60', label: '1分' },
  { value: '120', label: '2分' },
  { value: '300', label: '5分' },
  { value: '600', label: '10分' },
] as const;

type CharacterPanelRequestGate = {
  mount(): void;
  unmount(): void;
  begin(): number;
  isCurrent(generation: number): boolean;
};

export function createCharacterPanelRequestGate(): CharacterPanelRequestGate {
  let mounted = false;
  let generation = 0;
  return {
    mount() {
      mounted = true;
      generation += 1;
    },
    unmount() {
      mounted = false;
      generation += 1;
    },
    begin() {
      generation += 1;
      return generation;
    },
    isCurrent(requestGeneration) {
      return mounted && requestGeneration === generation;
    },
  };
}

/** Character controls and global expression behavior settings. */
export function CharacterPanel() {
  const requestGate = useRef(createCharacterPanelRequestGate());
  const [manifest, setManifest] = useState<CharacterManifestDto | null>(null);
  const [settings, setSettings] = useState<CharacterSettingsDto | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [reloadGeneration, setReloadGeneration] = useState(0);
  const [savingTimeout, setSavingTimeout] = useState(false);

  useEffect(() => {
    const gate = requestGate.current;
    gate.mount();
    return () => gate.unmount();
  }, []);

  useEffect(() => {
    let cancelled = false;
    Promise.all([
      invoke<CharacterManifestDto>('get_character_manifest'),
      invoke<CharacterSettingsDto>('get_character_settings'),
    ])
      .then(([loadedManifest, loadedSettings]) => {
        if (cancelled) return;
        setManifest(loadedManifest);
        setSettings(loadedSettings);
        setMessage(null);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setManifest(null);
        setSettings(null);
        setMessage(`キャラクターモデルを読み込めません: ${String(error)}`);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadGeneration]);

  const applyExpression = (name: string) => {
    if (name === '') return;
    invoke('set_expression', { name }).catch((error: unknown) => {
      setMessage(`表情を適用できません: ${String(error)}`);
    });
  };

  const playMotion = (group: string) => {
    invoke('start_motion', { group }).catch((error: unknown) => {
      setMessage(`モーションを再生できません: ${String(error)}`);
    });
  };

  const saveIdleTimeout = (selected: string) => {
    if (settings === null) return;
    const timeoutSeconds = selected === 'never' ? null : Number(selected);
    const requestGeneration = requestGate.current.begin();
    setSavingTimeout(true);
    invoke<CharacterSettingsDto>('set_expression_idle_timeout', {
      timeoutSeconds,
    })
      .then((saved) => {
        if (!requestGate.current.isCurrent(requestGeneration)) return;
        setSettings(saved);
        setMessage(null);
      })
      .catch((error: unknown) => {
        if (!requestGate.current.isCurrent(requestGeneration)) return;
        setMessage(`表情の復帰時間を保存できません: ${String(error)}`);
      })
      .finally(() => {
        if (!requestGate.current.isCurrent(requestGeneration)) return;
        setSavingTimeout(false);
      });
  };

  const expressions = manifest?.renderer.kind === 'static_image'
    ? manifest.renderer.expressions.map((expression) => expression.name)
    : (manifest?.renderer.expressions ?? []);
  const motionGroups = manifest?.renderer.kind === 'live2d'
    ? manifest.renderer.motion_groups
    : [];
  const timeoutValue = settings?.expression_idle_timeout_seconds === null
    ? 'never'
    : String(settings?.expression_idle_timeout_seconds ?? 20);

  return (
    <section aria-label="キャラクター設定">
      <h2>キャラクター</h2>
      {message !== null && <p role="alert">{message}</p>}
      {(manifest === null || settings === null) && (
        <button type="button" onClick={() => setReloadGeneration((value) => value + 1)}>
          再読み込み
        </button>
      )}
      {manifest !== null && settings !== null && (
        <>
          <div>
            <label htmlFor="expression-select">表情</label>
            <select
              id="expression-select"
              defaultValue=""
              onChange={(event) => applyExpression(event.target.value)}
            >
              <option value="" disabled>表情を選択</option>
              {expressions.map((name) => (
                <option key={name} value={name}>{name}</option>
              ))}
            </select>
          </div>
          {motionGroups.length > 0 && (
            <div>
              <h3>モーション</h3>
              <ul>
                {motionGroups.map((group) => (
                  <li key={group.name}>
                    <button type="button" onClick={() => playMotion(group.name)}>
                      {group.name} を再生
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <div>
            <label htmlFor="expression-idle-timeout">
              表情をデフォルトに戻す時間
            </label>
            <select
              id="expression-idle-timeout"
              value={timeoutValue}
              disabled={savingTimeout}
              onChange={(event) => saveIdleTimeout(event.target.value)}
            >
              {IDLE_TIMEOUT_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
        </>
      )}
    </section>
  );
}
