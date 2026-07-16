import type { CharacterManifestDto, PersonaProfileDto } from '@parallel-world/contracts';
import { DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';

type TraitKey =
  | 'initiative'
  | 'closeness'
  | 'humor'
  | 'response_length'
  | 'emotional_expression'
  | 'reaction_interval'
  | 'machiavellianism'
  | 'narcissism'
  | 'psychopathy';

type TraitDefinition = {
  key: TraitKey;
  label: string;
  low: string;
  high: string;
};

const GENERAL_TRAITS: TraitDefinition[] = [
  { key: 'initiative', label: '積極性', low: '受け身', high: '積極的' },
  { key: 'closeness', label: '親密度', low: '距離を保つ', high: '親密' },
  { key: 'humor', label: 'ユーモア', low: '真面目', high: '冗談好き' },
  { key: 'response_length', label: '応答の長さ', low: '簡潔', high: '詳しい' },
  { key: 'emotional_expression', label: '感情表現', low: '控えめ', high: '豊か' },
  { key: 'reaction_interval', label: '反応間隔', low: 'ゆったり', high: '頻繁' },
];

const DARK_TRAITS: TraitDefinition[] = [
  { key: 'machiavellianism', label: 'マキャベリズム', low: '率直', high: '操作的' },
  { key: 'narcissism', label: 'ナルシシズム', low: '謙虚', high: '自己中心的' },
  { key: 'psychopathy', label: 'サイコパシー', low: '共感的', high: '冷淡' },
];

const DISCLAIMER = '本機能は、娯楽上のキャラクター表現を調整するものであり、心理診断、治療または医療上の助言を目的とするものではありません。生成内容は予測できず、不快感や精神的負担を生じる場合があります。内容を鵜呑みにせず、利用の継続はご自身で判断してください。強い苦痛を感じた場合は、直ちに本設定を無効化し、利用を中止してください。本表示は、法令上認められない責任まで免除するものではありません。';

function TraitSlider({
  definition,
  value,
  disabled,
  onChange,
}: {
  definition: TraitDefinition;
  value: number;
  disabled: boolean;
  onChange: (value: number) => void;
}) {
  const id = `personality-${definition.key}`;
  return (
    <div className="personality-slider">
      <div className="personality-slider__heading">
        <label htmlFor={id}>{definition.label}</label>
        <output htmlFor={id}>{value}</output>
      </div>
      <input
        id={id}
        type="range"
        min="0"
        max="100"
        step="1"
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <div className="personality-slider__scale" aria-hidden="true">
        <span>0 · {definition.low}</span><span>基準 50</span><span>{definition.high} · 100</span>
      </div>
    </div>
  );
}

export function PersonalityPanel() {
  const [profile, setProfile] = useState<PersonaProfileDto | null>(null);
  const [characterName, setCharacterName] = useState<string | null>(null);
  const [loadGeneration, setLoadGeneration] = useState(0);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ kind: 'error' | 'success'; text: string } | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);

  useEffect(() => subscribeEvent('character-settings-changed', () => {
    setLoadGeneration((current) => current + 1);
  }), []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setProfile(null);
    setCharacterName(null);
    setMessage(null);
    void invoke<CharacterManifestDto>('get_character_manifest')
      .then(async (manifest) => {
        const loaded = await invoke<PersonaProfileDto>('get_persona_profile', {
          characterId: manifest.id,
        });
        if (!cancelled) {
          setCharacterName(manifest.display_name);
          setProfile(loaded);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) setMessage({ kind: 'error', text: `性格設定を読み込めません: ${String(error)}` });
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [loadGeneration]);

  const update = (patch: Partial<PersonaProfileDto>) => {
    setProfile((current) => current ? { ...current, ...patch } : current);
    setMessage(null);
  };

  const requestIntenseExpression = (enabled: boolean) => {
    if (!enabled) {
      update({
        allow_intense_dark_expression: false,
        dark_expression_acknowledgement_version: null,
      });
      return;
    }
    setAcknowledged(false);
    setConfirmOpen(true);
  };

  const confirmIntenseExpression = () => {
    if (!acknowledged) return;
    update({
      allow_intense_dark_expression: true,
      dark_expression_acknowledgement_version: DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION,
    });
    setConfirmOpen(false);
    setAcknowledged(false);
  };

  const reset = () => {
    if (!profile) return;
    const defaults = Object.fromEntries(
      [...GENERAL_TRAITS, ...DARK_TRAITS].map(({ key }) => [key, 50]),
    ) as Pick<PersonaProfileDto, TraitKey>;
    update({
      ...defaults,
      allow_intense_dark_expression: false,
      dark_expression_acknowledgement_version: null,
    });
  };

  const save = async () => {
    if (!profile || saving) return;
    setSaving(true);
    setMessage(null);
    try {
      const saved = await invoke<PersonaProfileDto>('set_persona_profile', { profile });
      if (saved) setProfile(saved);
      setMessage({ kind: 'success', text: '保存しました。次のメッセージから適用されます。' });
    } catch (error) {
      setMessage({ kind: 'error', text: `性格設定を保存できません: ${String(error)}` });
    } finally {
      setSaving(false);
    }
  };

  const renderSliders = (definitions: TraitDefinition[]) => definitions.map((definition) => (
    <TraitSlider
      key={definition.key}
      definition={definition}
      value={profile?.[definition.key] ?? 50}
      disabled={loading || saving}
      onChange={(value) => update({ [definition.key]: value })}
    />
  ));

  return (
    <section className="personality-panel" aria-label="性格設定" aria-busy={loading || saving}>
      <div className="personality-panel__heading">
        <div><p className="eyebrow">Personality</p><h2>{characterName ? `${characterName}の性格` : '性格'}</h2></div>
        {profile ? <span className="character-scope">キャラクター別設定</span> : null}
      </div>
      {message?.kind === 'error' ? <p role="alert">{message.text}</p> : null}
      {message?.kind === 'success' ? <p role="status">{message.text}</p> : null}
      {loading ? <p role="status">性格設定を読み込み中…</p> : null}

      {profile ? (
        <>
          <fieldset className="personality-group">
            <legend>一般的な性格</legend>
            <div className="personality-grid">{renderSliders(GENERAL_TRAITS)}</div>
          </fieldset>

          <fieldset className="personality-group personality-group--dark">
            <legend>ダークトライアド</legend>
            <p className="dark-triad-warning" role="note">
              この性格値を高くすると、攻撃的・操作的・共感性の低い応答が増え、あなたを傷つけたり、トラウマを想起させたりする可能性があります。
            </p>
            <div className="personality-grid">{renderSliders(DARK_TRAITS)}</div>
            <div className="intense-expression-control">
              <label>
                <input
                  type="checkbox"
                  aria-label="強いダーク表現を許可"
                  checked={profile.allow_intense_dark_expression}
                  disabled={saving}
                  onChange={(event) => requestIntenseExpression(event.target.checked)}
                />
                <span><strong>強いダーク表現を許可</strong><small>自己責任でのみ有効にしてください</small></span>
              </label>
              <p className="dark-triad-warning">有効にすると心理的負担の強い表現が増える可能性があります。基本的な安全保護は維持されます。</p>
            </div>
          </fieldset>

          <p className="personality-disclaimer">{DISCLAIMER}</p>
          <div className="personality-actions">
            <button type="button" className="secondary-button" disabled={saving} onClick={reset}>基準値に戻す</button>
            <button type="button" disabled={saving} onClick={() => void save()}>{saving ? '保存中…' : '保存'}</button>
          </div>
        </>
      ) : null}

      {confirmOpen ? (
        <div className="confirmation-backdrop" role="presentation">
          <section className="confirmation-dialog" role="dialog" aria-modal="true" aria-labelledby="intense-confirm-title">
            <p className="eyebrow">Safety confirmation</p>
            <h3 id="intense-confirm-title">強いダーク表現の確認</h3>
            <p className="dark-triad-warning">この設定は心理的負担の強い表現を増やす可能性があります。LLM提供元およびParallel Worldの基本的な安全保護は解除されません。</p>
            <label className="confirmation-check">
              <input type="checkbox" checked={acknowledged} onChange={(event) => setAcknowledged(event.target.checked)} />
              リスクを理解し、自己責任で有効にする
            </label>
            <div className="confirmation-actions">
              <button type="button" className="secondary-button" onClick={() => setConfirmOpen(false)}>キャンセル</button>
              <button type="button" disabled={!acknowledged} onClick={confirmIntenseExpression}>自己責任で有効にする</button>
            </div>
          </section>
        </div>
      ) : null}
    </section>
  );
}
