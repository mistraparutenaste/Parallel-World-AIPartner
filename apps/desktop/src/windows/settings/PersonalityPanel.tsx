import type {
  CharacterManifestDto,
  DarkExpressionSafetyChangedEventDto,
  DarkExpressionSafetySettingsDto,
  PersonaProfileDto,
} from '@parallel-world/contracts';
import {
  DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION,
  DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
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
  | 'psychopathy'
  | 'sadism';

type TraitDefinition = {
  key: TraitKey;
  label: string;
  low?: string;
  high?: string;
  dark?: boolean;
};

type NarrativeDraft = {
  name: string;
  first_person_pronoun: string;
  user_name: string;
  user_address: string;
  relationship: string;
  speaking_style: string;
  interests: string;
  dislikes: string;
  values: string;
  background: string;
  boundaries: string;
  free_text: string;
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
  { key: 'machiavellianism', label: 'マキャベリズム', dark: true },
  { key: 'narcissism', label: 'ナルシシズム', dark: true },
  { key: 'psychopathy', label: 'サイコパシー', dark: true },
  { key: 'sadism', label: 'サディズム', dark: true },
];

const DISCLAIMER = '本機能は、娯楽上のキャラクター表現を調整するものであり、心理診断、治療または医療上の助言を目的とするものではありません。生成内容は予測できず、不快感や精神的負担を生じる場合があります。内容を鵜呑みにせず、利用の継続はご自身で判断してください。強い苦痛を感じた場合は、直ちに本設定を無効化し、利用を中止してください。本表示は、法令上認められない責任まで免除するものではありません。';
const SAFEWORD_WARNING = 'セーフワードが設定されていません。強いダーク表現は利用できますが、本当に停止したい意思を確実に伝えるため、設定を推奨します。未設定でも停止操作や「強いダーク表現」の無効化は利用できます。';

function listToDraft(values: string[]) {
  return values.join('、');
}

function parseDraftList(value: string) {
  return value
    .split(/[\n,，、]/)
    .map((item) => item.trim())
    .filter((item) => item !== '');
}

function narrativeFromProfile(profile: PersonaProfileDto): NarrativeDraft {
  return {
    name: profile.name,
    first_person_pronoun: profile.first_person_pronoun,
    user_name: profile.user_name,
    user_address: profile.user_address,
    relationship: profile.relationship,
    speaking_style: profile.speaking_style,
    interests: listToDraft(profile.interests),
    dislikes: listToDraft(profile.dislikes),
    values: listToDraft(profile.values),
    background: profile.background,
    boundaries: listToDraft(profile.boundaries),
    free_text: profile.free_text,
  };
}

function profileWithNarrative(
  profile: PersonaProfileDto,
  draft: NarrativeDraft,
): PersonaProfileDto {
  return {
    ...profile,
    name: draft.name,
    first_person_pronoun: draft.first_person_pronoun,
    user_name: draft.user_name,
    user_address: draft.user_address,
    relationship: draft.relationship,
    speaking_style: draft.speaking_style,
    interests: parseDraftList(draft.interests),
    dislikes: parseDraftList(draft.dislikes),
    values: parseDraftList(draft.values),
    background: draft.background,
    boundaries: parseDraftList(draft.boundaries),
    free_text: draft.free_text,
  };
}

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
        <span>0 · {definition.dark ? '低い' : definition.low}</span>
        <span>基準 50</span>
        <span>{definition.dark ? '高い' : definition.high} · 100</span>
      </div>
    </div>
  );
}

function NarrativeField({
  label,
  value,
  multiline = false,
  full = false,
  disabled,
  onChange,
}: {
  label: string;
  value: string;
  multiline?: boolean;
  full?: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className={full ? 'personality-field personality-field--full' : 'personality-field'}>
      <span>{label}</span>
      {multiline ? (
        <textarea
          value={value}
          rows={3}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
        />
      ) : (
        <input
          type="text"
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </label>
  );
}

export function PersonalityPanel() {
  const [profile, setProfile] = useState<PersonaProfileDto | null>(null);
  const [narrative, setNarrative] = useState<NarrativeDraft | null>(null);
  const [safety, setSafety] = useState<DarkExpressionSafetySettingsDto | null>(null);
  const [safeWordDraft, setSafeWordDraft] = useState('');
  const [characterName, setCharacterName] = useState<string | null>(null);
  const [loadGeneration, setLoadGeneration] = useState(0);
  const [loading, setLoading] = useState(true);
  const [savingProfile, setSavingProfile] = useState(false);
  const [savingSafety, setSavingSafety] = useState(false);
  const [message, setMessage] = useState<{ kind: 'error' | 'status'; text: string } | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const profileSaveInFlight = useRef(false);
  const safetySaveInFlight = useRef(false);

  useEffect(() => subscribeEvent('character-settings-changed', () => {
    setLoadGeneration((current) => current + 1);
  }), []);

  useEffect(() => subscribeEvent<DarkExpressionSafetyChangedEventDto>(
    DARK_EXPRESSION_SAFETY_CHANGED_EVENT,
    (event) => {
      setSafety(event.settings);
      setSafeWordDraft(event.settings.safe_word ?? '');
    },
  ), []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setProfile(null);
    setNarrative(null);
    setCharacterName(null);
    setMessage(null);
    void invoke<CharacterManifestDto>('get_character_manifest')
      .then(async (manifest) => {
        const [profileResult, safetyResult] = await Promise.allSettled([
          invoke<PersonaProfileDto>('get_persona_profile', {
            characterId: manifest.id,
          }),
          invoke<DarkExpressionSafetySettingsDto>('get_dark_expression_safety_settings'),
        ]);
        if (cancelled) return;
        setCharacterName(manifest.display_name);
        if (profileResult.status === 'fulfilled' && profileResult.value) {
          setProfile(profileResult.value);
          setNarrative(narrativeFromProfile(profileResult.value));
        } else {
          setMessage({
            kind: 'error',
            text: '性格設定を読み込めません。診断を確認してください。',
          });
        }
        if (safetyResult.status === 'fulfilled' && safetyResult.value) {
          setSafety(safetyResult.value);
          setSafeWordDraft(safetyResult.value.safe_word ?? '');
        } else {
          setSafety(null);
          setSafeWordDraft('');
          setMessage({
            kind: 'error',
            text: '安全設定を読み込めません。強いダーク表現は停止した状態で扱います。',
          });
        }
      })
      .catch(() => {
        if (!cancelled) {
          setMessage({
            kind: 'error',
            text: '性格設定を読み込めません。診断を確認してください。',
          });
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [loadGeneration]);

  const persistProfile = async (
    next: PersonaProfileDto,
    optimistic: boolean,
  ) => {
    if (!profile || profileSaveInFlight.current) return false;
    const previous = profile;
    profileSaveInFlight.current = true;
    setSavingProfile(true);
    setMessage(null);
    if (optimistic) setProfile(next);
    try {
      const saved = await invoke<PersonaProfileDto>('set_persona_profile', { profile: next });
      setProfile(saved ?? next);
      return true;
    } catch {
      setProfile(previous);
      setMessage({
        kind: 'error',
        text: '性格設定を変更できません。少し待って、もう一度試してください。',
      });
      return false;
    } finally {
      profileSaveInFlight.current = false;
      setSavingProfile(false);
    }
  };

  const updateTrait = (key: TraitKey, value: number) => {
    if (!profile) return;
    void persistProfile({ ...profile, [key]: value }, true);
  };

  const narrativeDirty = profile && narrative
    ? JSON.stringify(narrativeFromProfile(profile)) !== JSON.stringify(narrative)
    : false;

  const applyNarrative = async () => {
    if (!profile || !narrative) return;
    const next = profileWithNarrative(profile, narrative);
    const saved = await persistProfile(next, false);
    if (saved) {
      setNarrative(narrativeFromProfile(next));
      setMessage({ kind: 'status', text: 'この子についての内容を適用しました。' });
    }
  };

  const requestIntenseExpression = (enabled: boolean) => {
    if (!profile) return;
    if (!enabled) {
      void persistProfile({
        ...profile,
        allow_intense_dark_expression: false,
        dark_expression_acknowledgement_version: null,
      }, true);
      return;
    }
    setAcknowledged(false);
    setConfirmOpen(true);
  };

  const confirmIntenseExpression = async () => {
    if (!profile || !acknowledged) return;
    const saved = await persistProfile({
      ...profile,
      allow_intense_dark_expression: true,
      dark_expression_acknowledgement_version: DARK_EXPRESSION_ACKNOWLEDGEMENT_VERSION,
    }, true);
    if (saved) {
      setConfirmOpen(false);
      setAcknowledged(false);
    }
  };

  const resetTraits = () => {
    if (!profile) return;
    const defaults = Object.fromEntries(
      [...GENERAL_TRAITS, ...DARK_TRAITS].map(({ key }) => [key, 50]),
    ) as Pick<PersonaProfileDto, TraitKey>;
    void persistProfile({
      ...profile,
      ...defaults,
      allow_intense_dark_expression: false,
      dark_expression_acknowledgement_version: null,
    }, true);
  };

  const safeWordDirty = safeWordDraft !== (safety?.safe_word ?? '');

  const applySafeWord = async () => {
    if (!safety || safetySaveInFlight.current) return;
    safetySaveInFlight.current = true;
    setSavingSafety(true);
    setMessage(null);
    try {
      const saved = await invoke<DarkExpressionSafetySettingsDto>('set_safe_word', {
        safeWord: safeWordDraft.trim() || null,
      });
      if (saved) {
        setSafety(saved);
        setSafeWordDraft(saved.safe_word ?? '');
      }
    } catch {
      setSafeWordDraft(safety.safe_word ?? '');
      setMessage({
        kind: 'error',
        text: 'セーフワードを変更できません。少し待って、もう一度試してください。',
      });
    } finally {
      safetySaveInFlight.current = false;
      setSavingSafety(false);
    }
  };

  const resumeDarkExpression = async () => {
    if (!safety || safetySaveInFlight.current) return;
    safetySaveInFlight.current = true;
    setSavingSafety(true);
    setMessage(null);
    try {
      const saved = await invoke<DarkExpressionSafetySettingsDto>('resume_dark_expression');
      if (saved) {
        setSafety(saved);
        setSafeWordDraft(saved.safe_word ?? '');
      }
    } catch {
      setMessage({
        kind: 'error',
        text: 'ダーク表現を再開できません。停止した状態を維持しています。',
      });
    } finally {
      safetySaveInFlight.current = false;
      setSavingSafety(false);
    }
  };

  const renderSliders = (definitions: TraitDefinition[]) => definitions.map((definition) => (
    <TraitSlider
      key={definition.key}
      definition={definition}
      value={profile?.[definition.key] ?? 50}
      disabled={loading || savingProfile}
      onChange={(value) => updateTrait(definition.key, value)}
    />
  ));

  return (
    <section
      className="personality-panel"
      aria-label="性格設定"
      aria-busy={loading || savingProfile || savingSafety}
    >
      <div className="personality-panel__heading">
        <div>
          <p className="eyebrow">Personality</p>
          <h2>{characterName ? `${characterName}の性格` : '性格'}</h2>
        </div>
        {profile ? <span className="character-scope">キャラクター別設定</span> : null}
      </div>
      {message?.kind === 'error' ? <p className="personality-message" role="alert">{message.text}</p> : null}
      {message?.kind === 'status' ? <p className="personality-message" role="status">{message.text}</p> : null}
      {loading ? <p role="status">性格設定を読み込み中…</p> : null}

      {profile && narrative ? (
        <>
          <fieldset className="personality-group personality-group--narrative" aria-label="この子について">
            <legend>この子について</legend>
            <div className="personality-profile-groups">
              <section
                className="personality-profile-section personality-profile-section--identity"
                aria-labelledby="personality-profile-identity"
              >
                <div className="personality-profile-section__heading">
                  <span aria-hidden="true" />
                  <h3 id="personality-profile-identity">基本情報</h3>
                </div>
                <div className="personality-fields">
                  <NarrativeField label="名前" value={narrative.name} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, name: value })} />
                  <NarrativeField label="一人称" value={narrative.first_person_pronoun} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, first_person_pronoun: value })} />
                  <NarrativeField label="ユーザーの名前" value={narrative.user_name} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, user_name: value })} />
                  <NarrativeField label="ユーザーの呼び方" value={narrative.user_address} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, user_address: value })} />
                  <NarrativeField label="関係性" value={narrative.relationship} full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, relationship: value })} />
                </div>
              </section>

              <section
                className="personality-profile-section personality-profile-section--voice"
                aria-labelledby="personality-profile-voice"
              >
                <div className="personality-profile-section__heading">
                  <span aria-hidden="true" />
                  <h3 id="personality-profile-voice">話し方と嗜好</h3>
                </div>
                <div className="personality-fields">
                  <NarrativeField label="話し方" value={narrative.speaking_style} full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, speaking_style: value })} />
                  <NarrativeField label="興味" value={narrative.interests} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, interests: value })} />
                  <NarrativeField label="苦手なもの" value={narrative.dislikes} disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, dislikes: value })} />
                  <NarrativeField label="価値観" value={narrative.values} full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, values: value })} />
                </div>
              </section>

              <section
                className="personality-profile-section personality-profile-section--story"
                aria-labelledby="personality-profile-story"
              >
                <div className="personality-profile-section__heading">
                  <span aria-hidden="true" />
                  <h3 id="personality-profile-story">背景と境界</h3>
                </div>
                <div className="personality-fields">
                  <NarrativeField label="背景" value={narrative.background} multiline full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, background: value })} />
                  <NarrativeField label="境界" value={narrative.boundaries} multiline full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, boundaries: value })} />
                  <NarrativeField label="自由記述" value={narrative.free_text} multiline full disabled={savingProfile} onChange={(value) => setNarrative({ ...narrative, free_text: value })} />
                </div>
              </section>
            </div>
            {narrativeDirty ? (
              <div className="personality-inline-actions">
                <button
                  type="button"
                  className="secondary-button"
                  disabled={savingProfile}
                  onClick={() => setNarrative(narrativeFromProfile(profile))}
                >
                  元に戻す
                </button>
                <button
                  type="button"
                  disabled={savingProfile}
                  onClick={() => void applyNarrative()}
                >
                  この子についてを適用
                </button>
              </div>
            ) : null}
          </fieldset>

          <fieldset className="personality-group" aria-label="会話の傾向">
            <legend>会話の傾向</legend>
            <div className="personality-grid">{renderSliders(GENERAL_TRAITS)}</div>
          </fieldset>

          <fieldset className="personality-group personality-group--dark" aria-label="ダーク傾向">
            <legend>ダーク傾向</legend>
            <p className="dark-triad-warning" role="note">
              この性格値を高くすると、攻撃的・操作的・共感性の低い応答が増え、あなたを傷つけたり、トラウマを想起させたりする可能性があります。
            </p>
            <div className="personality-grid">{renderSliders(DARK_TRAITS)}</div>

            <div className="safe-word-control">
              <label htmlFor="personality-safe-word">セーフワード（推奨）</label>
              <p>この言葉を送ると、ロール中でも会話生成と音声をすぐに停止します。</p>
              <input
                id="personality-safe-word"
                type="text"
                value={safeWordDraft}
                disabled={!safety || savingSafety}
                aria-describedby={!safeWordDraft.trim() ? 'safe-word-warning' : undefined}
                onChange={(event) => setSafeWordDraft(event.target.value)}
              />
              {!safeWordDraft.trim() ? (
                <p id="safe-word-warning" className="safe-word-warning" role="status">
                  {SAFEWORD_WARNING}
                </p>
              ) : null}
              {safeWordDirty ? (
                <div className="personality-inline-actions">
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={savingSafety}
                    onClick={() => setSafeWordDraft(safety?.safe_word ?? '')}
                  >
                    元に戻す
                  </button>
                  <button
                    type="button"
                    disabled={savingSafety}
                    onClick={() => void applySafeWord()}
                  >
                    セーフワードを適用
                  </button>
                </div>
              ) : null}
            </div>

            {safety?.dark_expression_paused ? (
              <div className="dark-expression-paused" role="status">
                <strong>ダーク表現を停止しています。</strong>
                <button
                  type="button"
                  disabled={savingSafety}
                  onClick={() => void resumeDarkExpression()}
                >
                  ダーク表現を再開
                </button>
              </div>
            ) : null}

            <div className="intense-expression-control">
              <label>
                <input
                  type="checkbox"
                  role="switch"
                  aria-label="強いダーク表現を許可"
                  checked={profile.allow_intense_dark_expression}
                  disabled={savingProfile}
                  onChange={(event) => requestIntenseExpression(event.target.checked)}
                />
                <span>
                  <strong>強いダーク表現を許可</strong>
                  <small>自己責任でのみ有効にしてください</small>
                </span>
              </label>
              <p className="dark-triad-warning">
                有効にすると心理的負担の強い表現が増える可能性があります。基本的な安全保護は維持されます。
              </p>
            </div>
          </fieldset>

          <p className="personality-disclaimer">{DISCLAIMER}</p>
          <div className="personality-actions">
            <button
              type="button"
              className="secondary-button"
              disabled={savingProfile}
              onClick={resetTraits}
            >
              性格値を基準へ戻す
            </button>
          </div>
        </>
      ) : null}

      {confirmOpen ? (
        <div className="confirmation-backdrop" role="presentation">
          <section
            className="confirmation-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="intense-confirm-title"
          >
            <p className="eyebrow">Safety confirmation</p>
            <h3 id="intense-confirm-title">強いダーク表現の確認</h3>
            <p className="dark-triad-warning">
              この設定は心理的負担の強い表現を増やす可能性があります。LLM提供元およびParallel Worldの基本的な安全保護は解除されません。
            </p>
            {!safety?.safe_word ? <p className="safe-word-warning">{SAFEWORD_WARNING}</p> : null}
            <label className="confirmation-check">
              <input
                type="checkbox"
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
              />
              リスクを理解し、自己責任で有効にする
            </label>
            <div className="confirmation-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setConfirmOpen(false)}
              >
                キャンセル
              </button>
              <button
                type="button"
                disabled={!acknowledged || savingProfile}
                onClick={() => void confirmIntenseExpression()}
              >
                自己責任で有効にする
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </section>
  );
}
