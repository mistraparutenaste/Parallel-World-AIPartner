import type {
  CharacterManifestDto,
  CharacterRendererKindDto,
  CharacterSettingsDto,
  CharacterSetupDto,
  CharacterSourceStatusDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
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

const CHARACTER_SIZE_OPTIONS = Array.from(
  { length: 16 },
  (_, index) => 50 + index * 10,
);

const SOURCE_LABELS: Record<CharacterRendererKindDto, string> = {
  live2d: 'Live2D',
  static_image: '静止画',
};

type CharacterPanelRequestGate = {
  mount(): void;
  unmount(): void;
  begin(): number;
  isCurrent(generation: number): boolean;
};

type PanelMessage = { kind: 'error' | 'success'; text: string };

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

function CharacterSourceCard({
  source,
  busy,
  onSelectFile,
}: {
  source: CharacterSourceStatusDto;
  busy: boolean;
  onSelectFile: (kind: CharacterRendererKindDto) => void;
}) {
  const label = SOURCE_LABELS[source.kind];
  return (
    <article className="character-source-card" aria-label={`${label}の設定`}>
      <div className="character-source-card__heading">
        <h3>{label}</h3>
        <span className="character-source-state">
          {source.active ? '表示中' : source.configured ? '設定済み' : '未設定'}
        </span>
      </div>
      <dl className="character-source-details">
        <div><dt>表示名</dt><dd>{source.display_name ?? '未設定'}</dd></div>
        <div><dt>ファイル</dt><dd>{source.file_name ?? '未選択'}</dd></div>
      </dl>
      <button
        type="button"
        className="secondary-button"
        disabled={busy || !source.import_enabled}
        onClick={() => onSelectFile(source.kind)}
      >
        {label}ファイルを選択
      </button>
      {source.kind === 'live2d' && !source.import_enabled ? (
        <p className="character-source-note">
          任意のLive2D読み込みは開発ビルドでのみ利用できます。
        </p>
      ) : null}
    </article>
  );
}

/** Character source setup, controls, and global expression behavior settings. */
export function CharacterPanel() {
  const loadGate = useRef(createCharacterPanelRequestGate());
  const setupActionGate = useRef(createCharacterPanelRequestGate());
  const timeoutGate = useRef(createCharacterPanelRequestGate());
  const sizeGate = useRef(createCharacterPanelRequestGate());
  const [setup, setSetup] = useState<CharacterSetupDto | null>(null);
  const [manifest, setManifest] = useState<CharacterManifestDto | null>(null);
  const [settings, setSettings] = useState<CharacterSettingsDto | null>(null);
  const [message, setMessage] = useState<PanelMessage | null>(null);
  const [reloadGeneration, setReloadGeneration] = useState(0);
  const [loading, setLoading] = useState(true);
  const [setupBusy, setSetupBusy] = useState(false);
  const [savingTimeout, setSavingTimeout] = useState(false);
  const [savingSize, setSavingSize] = useState(false);

  useEffect(() => {
    const gates = [loadGate.current, setupActionGate.current, timeoutGate.current, sizeGate.current];
    for (const gate of gates) gate.mount();
    return () => {
      for (const gate of gates) gate.unmount();
    };
  }, []);

  useEffect(() => {
    setupActionGate.current.begin();
    timeoutGate.current.begin();
    sizeGate.current.begin();
    const requestGeneration = loadGate.current.begin();
    setLoading(true);
    setSetupBusy(false);
    setSavingTimeout(false);
    setSavingSize(false);
    void Promise.allSettled([
      invoke<CharacterSetupDto>('get_character_setup'),
      invoke<CharacterSettingsDto>('get_character_settings'),
      invoke<CharacterManifestDto>('get_character_manifest'),
    ]).then(([setupResult, settingsResult, manifestResult]) => {
      if (!loadGate.current.isCurrent(requestGeneration)) return;

      if (setupResult.status === 'fulfilled') setSetup(setupResult.value);
      else setSetup(null);
      if (settingsResult.status === 'fulfilled') setSettings(settingsResult.value);
      else setSettings(null);
      if (manifestResult.status === 'fulfilled') setManifest(manifestResult.value);
      else setManifest(null);

      if (setupResult.status === 'rejected') {
        setMessage({ kind: 'error', text: `キャラクター設定を読み込めません: ${String(setupResult.reason)}` });
      } else if (settingsResult.status === 'rejected') {
        setMessage({ kind: 'error', text: `キャラクター設定を読み込めません: ${String(settingsResult.reason)}` });
      } else if (
        manifestResult.status === 'rejected'
        && !String(manifestResult.reason).includes('selection_required')
      ) {
        setMessage({ kind: 'error', text: `キャラクターモデルを読み込めません: ${String(manifestResult.reason)}` });
      } else {
        setMessage(null);
      }
      setLoading(false);
    });
  }, [reloadGeneration]);

  const applyExpression = (name: string) => {
    if (name === '') return;
    invoke('set_expression', { name }).catch((error: unknown) => {
      setMessage({ kind: 'error', text: `表情を適用できません: ${String(error)}` });
    });
  };

  const playMotion = (group: string) => {
    invoke('start_motion', { group }).catch((error: unknown) => {
      setMessage({ kind: 'error', text: `モーションを再生できません: ${String(error)}` });
    });
  };

  const importSource = async (kind: CharacterRendererKindDto) => {
    if (setup === null || setupBusy) return;
    const source = kind === 'live2d' ? setup.live2d : setup.static_image;
    if (!source.import_enabled) return;
    loadGate.current.begin();
    timeoutGate.current.begin();
    const requestGeneration = setupActionGate.current.begin();
    setSetupBusy(true);
    setLoading(false);
    setMessage(null);
    try {
      const sourcePath = await open({
        multiple: false,
        filters: kind === 'live2d'
          ? [{ name: 'Live2Dモデル', extensions: ['model3.json', 'json'] }]
          : [{ name: '静止画', extensions: ['png', 'webp'] }],
      });
      if (!setupActionGate.current.isCurrent(requestGeneration)) return;
      if (sourcePath === null) return;
      const imported = await invoke<CharacterSetupDto>('import_character_asset', {
        kind,
        sourcePath,
      });
      if (!setupActionGate.current.isCurrent(requestGeneration)) return;
      setSetup(imported);
      const importedActiveRenderer = imported.active_renderer === kind;
      if (importedActiveRenderer) {
        setManifest(null);
        try {
          const activeManifest = await invoke<CharacterManifestDto>('get_character_manifest');
          if (!setupActionGate.current.isCurrent(requestGeneration)) return;
          setManifest(activeManifest);
        } catch (error) {
          if (!setupActionGate.current.isCurrent(requestGeneration)) return;
          setMessage({
            kind: 'error',
            text: `アセットは読み込みましたが、キャラクターモデルを更新できません: ${String(error)}`,
          });
          return;
        }
      }
      setMessage({
        kind: 'success',
        text: importedActiveRenderer
          ? '表示中のアセットを更新しました。'
          : 'アセットを読み込みました。トグルで切替できます。',
      });
    } catch (error) {
      if (!setupActionGate.current.isCurrent(requestGeneration)) return;
      setMessage({ kind: 'error', text: `キャラクターアセットを読み込めません: ${String(error)}` });
    } finally {
      if (setupActionGate.current.isCurrent(requestGeneration)) setSetupBusy(false);
    }
  };

  const switchRenderer = async (kind: CharacterRendererKindDto) => {
    if (setup === null || setupBusy || setup.active_renderer === kind) return;
    const source = kind === 'live2d' ? setup.live2d : setup.static_image;
    if (!source.configured) return;
    loadGate.current.begin();
    timeoutGate.current.begin();
    const requestGeneration = setupActionGate.current.begin();
    setSetupBusy(true);
    setLoading(false);
    setMessage(null);
    try {
      let switched: CharacterSetupDto;
      try {
        switched = await invoke<CharacterSetupDto>('set_active_character_renderer', { kind });
      } catch (error) {
        if (!setupActionGate.current.isCurrent(requestGeneration)) return;
        setMessage({ kind: 'error', text: `キャラクター形式を切り替えできません: ${String(error)}` });
        return;
      }
      if (!setupActionGate.current.isCurrent(requestGeneration)) return;
      setSetup(switched);
      setManifest(null);
      try {
        const activeManifest = await invoke<CharacterManifestDto>('get_character_manifest');
        if (!setupActionGate.current.isCurrent(requestGeneration)) return;
        setManifest(activeManifest);
      } catch (error) {
        if (!setupActionGate.current.isCurrent(requestGeneration)) return;
        setMessage({
          kind: 'error',
          text: `切り替えは完了しましたが、キャラクターモデルを読み込めません: ${String(error)}`,
        });
      }
    } finally {
      if (setupActionGate.current.isCurrent(requestGeneration)) setSetupBusy(false);
    }
  };

  const saveIdleTimeout = (selected: string) => {
    if (settings === null) return;
    const timeoutSeconds = selected === 'never' ? null : Number(selected);
    loadGate.current.begin();
    setupActionGate.current.begin();
    const requestGeneration = timeoutGate.current.begin();
    setSavingTimeout(true);
    invoke<CharacterSettingsDto>('set_expression_idle_timeout', { timeoutSeconds })
      .then((saved) => {
        if (!timeoutGate.current.isCurrent(requestGeneration)) return;
        setSettings(saved);
        setMessage(null);
      })
      .catch((error: unknown) => {
        if (!timeoutGate.current.isCurrent(requestGeneration)) return;
        setMessage({ kind: 'error', text: `表情の復帰時間を保存できません: ${String(error)}` });
      })
      .finally(() => {
        if (timeoutGate.current.isCurrent(requestGeneration)) setSavingTimeout(false);
      });
  };

  const saveCharacterSize = (selected: string) => {
    if (settings === null) return;
    const sizePercent = Number(selected);
    loadGate.current.begin();
    setupActionGate.current.begin();
    const requestGeneration = sizeGate.current.begin();
    setSavingSize(true);
    invoke<CharacterSettingsDto>('set_character_size', { sizePercent })
      .then((saved) => {
        if (!sizeGate.current.isCurrent(requestGeneration)) return;
        setSettings(saved);
        setMessage(null);
      })
      .catch((error: unknown) => {
        if (!sizeGate.current.isCurrent(requestGeneration)) return;
        setMessage({ kind: 'error', text: `キャラクターサイズを保存できません: ${String(error)}` });
      })
      .finally(() => {
        if (sizeGate.current.isCurrent(requestGeneration)) setSavingSize(false);
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
  const sizeValue = String(settings?.character_size_percent ?? 100);
  const controlsBusy = loading || setupBusy || savingTimeout || savingSize;

  return (
    <section className="character-panel" aria-label="キャラクター設定" aria-busy={controlsBusy}>
      <h2>キャラクター</h2>
      {message?.kind === 'error' ? <p role="alert">{message.text}</p> : null}
      {message?.kind === 'success' ? <p role="status">{message.text}</p> : null}

      {setup !== null ? (
        <div className="character-setup" aria-busy={controlsBusy}>
          <fieldset
            className="character-renderer-selector"
            role="radiogroup"
            aria-label="表示するキャラクター形式"
          >
            <legend>表示するキャラクター形式</legend>
            {([setup.live2d, setup.static_image] as const).map((source) => (
              <label key={source.kind}>
                <input
                  type="radio"
                  name="active-character-renderer"
                  value={source.kind}
                  checked={setup.active_renderer === source.kind}
                  disabled={controlsBusy || !source.configured}
                  onChange={() => void switchRenderer(source.kind)}
                />
                <span>{SOURCE_LABELS[source.kind]}</span>
              </label>
            ))}
          </fieldset>
          {controlsBusy ? <p className="character-setup-progress">処理中…</p> : null}
          <div className="character-source-grid">
            <CharacterSourceCard source={setup.live2d} busy={controlsBusy} onSelectFile={(kind) => void importSource(kind)} />
            <CharacterSourceCard source={setup.static_image} busy={controlsBusy} onSelectFile={(kind) => void importSource(kind)} />
          </div>
        </div>
      ) : null}

      {(setup === null || settings === null || manifest === null) ? (
        <button
          type="button"
          className="secondary-button"
          disabled={controlsBusy}
          onClick={() => setReloadGeneration((value) => value + 1)}
        >
          再読み込み
        </button>
      ) : null}

      {settings !== null ? (
        <div className="character-size-control">
          <label htmlFor="character-size">キャラクターサイズ</label>
          <select
            id="character-size"
            value={sizeValue}
            disabled={controlsBusy}
            onChange={(event) => saveCharacterSize(event.target.value)}
          >
            {CHARACTER_SIZE_OPTIONS.map((size) => (
              <option key={size} value={size}>{size}%</option>
            ))}
          </select>
        </div>
      ) : null}

      {manifest !== null && settings !== null ? (
        <div className="character-runtime-controls">
          <div>
            <label htmlFor="expression-select">表情</label>
            <select id="expression-select" defaultValue="" onChange={(event) => applyExpression(event.target.value)}>
              <option value="" disabled>表情を選択</option>
              {expressions.map((name) => <option key={name} value={name}>{name}</option>)}
            </select>
          </div>
          {motionGroups.length > 0 ? (
            <div>
              <h3>モーション</h3>
              <ul>
                {motionGroups.map((group) => (
                  <li key={group.name}>
                    <button type="button" onClick={() => playMotion(group.name)}>{group.name} を再生</button>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          <div>
            <label htmlFor="expression-idle-timeout">表情をデフォルトに戻す時間</label>
            <select
              id="expression-idle-timeout"
              value={timeoutValue}
              disabled={controlsBusy}
              onChange={(event) => saveIdleTimeout(event.target.value)}
            >
              {IDLE_TIMEOUT_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>{option.label}</option>
              ))}
            </select>
          </div>
        </div>
      ) : null}
    </section>
  );
}
