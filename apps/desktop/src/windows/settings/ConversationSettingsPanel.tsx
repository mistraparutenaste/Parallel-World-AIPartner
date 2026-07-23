import type {
  BehaviorSettingsChangedEventDto,
  BehaviorSettingsDto,
  ActiveModeDto,
  ActivityCollectionHealthEventDto,
  ActivitySessionPageDto,
  FrequencyPolicyDto,
  QuietHoursRuleDto,
} from '@parallel-world/contracts';
import { BEHAVIOR_SETTINGS_CHANGED_EVENT } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';

const ACTIVITY_COLLECTION_CONSENT_VERSION = 1;
const MAX_QUIET_HOURS_RULES = 32;

type FrequencyPreset = {
  label: string;
  sentence: string;
  policy: FrequencyPolicyDto;
};

const FREQUENCY_PRESETS: FrequencyPreset[] = [
  {
    label: 'かなり控えめ',
    sentence: '3時間以上あけて、1日最大2回まで',
    policy: {
      minimum_interval_minutes: 180,
      max_per_hour: 1,
      max_per_day: 2,
    },
  },
  {
    label: '控えめ',
    sentence: '90分以上あけて、1日最大4回まで',
    policy: {
      minimum_interval_minutes: 90,
      max_per_hour: 1,
      max_per_day: 4,
    },
  },
  {
    label: 'ほどよく',
    sentence: '30分以上あけて、1日最大8回まで',
    policy: {
      minimum_interval_minutes: 30,
      max_per_hour: 2,
      max_per_day: 8,
    },
  },
  {
    label: '多め',
    sentence: '15分以上あけて、1日最大16回まで',
    policy: {
      minimum_interval_minutes: 15,
      max_per_hour: 3,
      max_per_day: 16,
    },
  },
  {
    label: '頻繁',
    sentence: '5分以上あけて、1日最大32回まで',
    policy: {
      minimum_interval_minutes: 5,
      max_per_hour: 6,
      max_per_day: 32,
    },
  },
];

const DAYS = [
  { value: 0, short: '月', name: '月曜日' },
  { value: 1, short: '火', name: '火曜日' },
  { value: 2, short: '水', name: '水曜日' },
  { value: 3, short: '木', name: '木曜日' },
  { value: 4, short: '金', name: '金曜日' },
  { value: 5, short: '土', name: '土曜日' },
  { value: 6, short: '日', name: '日曜日' },
] as const;

const RETURN_OPTIONS = [5, 10, 20, 30, 60];
const LONG_SESSION_OPTIONS = [30, 60, 90, 120, 180];
const CATEGORY_CHANGE_OPTIONS = [5, 10, 20, 30, 60];

type Feedback = {
  kind: 'error' | 'status';
  text: string;
};

type UndoQuietHours = {
  rule: QuietHoursRuleDto;
  index: number;
};

function sameFrequency(left: FrequencyPolicyDto, right: FrequencyPolicyDto) {
  return left.minimum_interval_minutes === right.minimum_interval_minutes
    && left.max_per_hour === right.max_per_hour
    && left.max_per_day === right.max_per_day;
}

function frequencyIndex(frequency: FrequencyPolicyDto) {
  return FREQUENCY_PRESETS.findIndex(({ policy }) => sameFrequency(frequency, policy));
}

function closestFrequencyIndex(frequency: FrequencyPolicyDto) {
  let bestIndex = 0;
  let bestDistance = Number.POSITIVE_INFINITY;
  FREQUENCY_PRESETS.forEach(({ policy }, index) => {
    const distance = Math.abs(policy.minimum_interval_minutes - frequency.minimum_interval_minutes)
      + Math.abs(policy.max_per_hour - frequency.max_per_hour) * 30
      + Math.abs(policy.max_per_day - frequency.max_per_day) * 5;
    if (distance < bestDistance) {
      bestDistance = distance;
      bestIndex = index;
    }
  });
  return bestIndex;
}

function customFrequencySentence(frequency: FrequencyPolicyDto) {
  const interval = frequency.minimum_interval_minutes % 60 === 0
    ? `${frequency.minimum_interval_minutes / 60}時間`
    : `${frequency.minimum_interval_minutes}分`;
  return `${interval}以上あけて、1日最大${frequency.max_per_day}回まで`;
}

function makeQuietHoursRule(): QuietHoursRuleDto {
  const fallback = `quiet-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  return {
    rule_id: globalThis.crypto?.randomUUID?.() ?? fallback,
    enabled: true,
    days_of_week: [0, 1, 2, 3, 4],
    start_local_time: '23:00',
    end_local_time: '07:00',
  };
}

function nextLocalMidnightSeconds() {
  const now = new Date();
  return Math.floor(
    new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime() / 1000,
  );
}

function formatSnooze(until: number) {
  const target = new Date(until * 1000);
  const now = new Date(Date.now());
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  const isTomorrowMidnight = target.getTime() === tomorrow.getTime();
  if (isTomorrowMidnight) return '明日0:00まで話しかけません。';
  if (target.toDateString() === now.toDateString()) {
    return `${target.getHours()}:${String(target.getMinutes()).padStart(2, '0')}まで話しかけません。`;
  }
  return `${target.getMonth() + 1}月${target.getDate()}日 ${target.getHours()}:${String(target.getMinutes()).padStart(2, '0')}まで話しかけません。`;
}

function Switch({
  label,
  checked,
  disabled,
  describedBy,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  describedBy?: string | undefined;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="conversation-switch">
      <input
        type="checkbox"
        role="switch"
        aria-label={label}
        aria-describedby={describedBy}
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span aria-hidden="true">{checked ? 'ON' : 'OFF'}</span>
    </label>
  );
}

function SettingsSection({
  id,
  title,
  children,
}: {
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="conversation-settings-section" aria-labelledby={id}>
      <h2 id={id}>{title}</h2>
      <div className="conversation-settings-section__body">{children}</div>
    </section>
  );
}

function selectOptions(current: number, options: number[], suffix = '分') {
  const values = options.includes(current) ? options : [current, ...options];
  return values.map((value) => (
    <option key={value} value={value}>
      {options.includes(value) ? `${value}${suffix}` : `以前の設定（${value}分）`}
    </option>
  ));
}

export function ConversationSettingsPanel() {
  const [settings, setSettings] = useState<BehaviorSettingsDto | null>(null);
  const [activeMode, setActiveMode] = useState<ActiveModeDto | null>(null);
  const [collectionHealth, setCollectionHealth] = useState<ActivityCollectionHealthEventDto | null>(null);
  const [activityPage, setActivityPage] = useState<ActivitySessionPageDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [consentOpen, setConsentOpen] = useState(false);
  const [snoozeChoicesOpen, setSnoozeChoicesOpen] = useState(false);
  const [quietHoursError, setQuietHoursError] = useState<string | null>(null);
  const [undoQuietHours, setUndoQuietHours] = useState<UndoQuietHours | null>(null);
  const undoTimer = useRef<number | null>(null);
  const saveInFlight = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void invoke<BehaviorSettingsDto>('get_behavior_settings')
      .then((loaded) => {
        if (!cancelled && loaded) setSettings(loaded);
      })
      .catch(() => {
        if (!cancelled) {
          setFeedback({
            kind: 'error',
            text: '会話設定を読み込めません。診断を確認してください。',
          });
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    const unsubscribe = subscribeEvent<BehaviorSettingsChangedEventDto>(
      BEHAVIOR_SETTINGS_CHANGED_EVENT,
      (event) => {
        setSettings(event.settings);
        setFeedback(null);
      },
    );
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void Promise.all([
        invoke<ActiveModeDto>('get_active_mode'),
        invoke<ActivityCollectionHealthEventDto>('get_activity_collection_health'),
        invoke<ActivitySessionPageDto>('list_activity_sessions', { limit: 5, beforeId: null }),
      ]).then(([mode, health, activity]) => {
        if (cancelled) return;
        setActiveMode(mode);
        setCollectionHealth(health);
        setActivityPage(activity);
      }).catch(() => {
        if (!cancelled) {
          setCollectionHealth({
            schema_version: 1,
            status: 'degraded',
            last_activity_at: null,
            message: null,
          });
        }
      });
    };
    refresh();
    const refreshTimer = window.setInterval(refresh, 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(refreshTimer);
    };
  }, []);

  useEffect(() => () => {
    if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
  }, []);

  const persist = async (
    next: BehaviorSettingsDto,
    {
      optimistic = true,
      failure = '今は会話設定を変更できません。少し待って、もう一度試してください。',
    }: { optimistic?: boolean; failure?: string } = {},
  ) => {
    if (!settings || saveInFlight.current) return false;
    const normalized = next.proactive_snoozed_until !== null
      && next.proactive_snoozed_until <= Math.floor(Date.now() / 1000)
      ? { ...next, proactive_snoozed_until: null }
      : next;
    const previous = settings;
    saveInFlight.current = true;
    setSaving(true);
    setFeedback(null);
    if (optimistic) setSettings(normalized);
    try {
      const saved = await invoke<BehaviorSettingsDto>('set_behavior_settings', {
        settings: normalized,
      });
      setSettings(saved ?? normalized);
      return true;
    } catch {
      setSettings(previous);
      setFeedback({ kind: 'error', text: failure });
      return false;
    } finally {
      saveInFlight.current = false;
      setSaving(false);
    }
  };

  const updateTriggers = (patch: Partial<BehaviorSettingsDto['triggers']>) => {
    if (!settings) return;
    void persist({
      ...settings,
      triggers: { ...settings.triggers, ...patch },
    });
  };

  const updateQuietHours = (
    index: number,
    patch: Partial<QuietHoursRuleDto>,
  ) => {
    if (!settings) return;
    const current = settings.quiet_hours[index];
    if (!current) return;
    const nextRule = { ...current, ...patch };
    if (nextRule.days_of_week.length === 0) {
      setQuietHoursError('曜日を1つ以上選んでください。');
      return;
    }
    const validLocalTime = /^(?:[01]\d|2[0-3]):[0-5]\d$/;
    if (
      !validLocalTime.test(nextRule.start_local_time)
      || !validLocalTime.test(nextRule.end_local_time)
    ) {
      setQuietHoursError('時刻は24時間表記で選んでください。');
      return;
    }
    if (nextRule.start_local_time === nextRule.end_local_time) {
      setQuietHoursError('開始時刻と終了時刻は別の時刻にしてください。');
      return;
    }
    setQuietHoursError(null);
    const quietHours = settings.quiet_hours.map((rule, ruleIndex) => (
      ruleIndex === index ? nextRule : rule
    ));
    void persist({ ...settings, quiet_hours: quietHours });
  };

  const addQuietHours = () => {
    if (!settings || settings.quiet_hours.length >= MAX_QUIET_HOURS_RULES) return;
    setQuietHoursError(null);
    void persist({
      ...settings,
      quiet_hours: [...settings.quiet_hours, makeQuietHoursRule()],
    });
  };

  const deleteQuietHours = async (index: number) => {
    if (!settings) return;
    const rule = settings.quiet_hours[index];
    if (!rule) return;
    const next = {
      ...settings,
      quiet_hours: settings.quiet_hours.filter((_, ruleIndex) => ruleIndex !== index),
    };
    const saved = await persist(next);
    if (!saved) return;
    if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
    setUndoQuietHours({ rule, index });
    undoTimer.current = window.setTimeout(() => {
      setUndoQuietHours(null);
      undoTimer.current = null;
    }, 5_000);
  };

  const restoreQuietHours = async () => {
    if (!settings || !undoQuietHours) return;
    const quietHours = [...settings.quiet_hours];
    quietHours.splice(
      Math.min(undoQuietHours.index, quietHours.length),
      0,
      undoQuietHours.rule,
    );
    const saved = await persist({ ...settings, quiet_hours: quietHours });
    if (saved) {
      setUndoQuietHours(null);
      if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
      undoTimer.current = null;
    }
  };

  const setSnooze = (until: number | null) => {
    if (!settings) return;
    setSnoozeChoicesOpen(false);
    void persist({ ...settings, proactive_snoozed_until: until });
  };

  const requestCollection = (enabled: boolean) => {
    if (!settings) return;
    if (!enabled) {
      setConsentOpen(false);
      void persist({ ...settings, collection_enabled: false });
      return;
    }
    if (
      settings.consent === 'accepted'
      && settings.consent_version === ACTIVITY_COLLECTION_CONSENT_VERSION
    ) {
      void persist({ ...settings, collection_enabled: true });
      return;
    }
    setConsentOpen(true);
    setFeedback(null);
  };

  const acceptCollection = async () => {
    if (!settings) return;
    const saved = await persist(
      {
        ...settings,
        consent: 'accepted',
        consent_version: ACTIVITY_COLLECTION_CONSENT_VERSION,
        collection_enabled: true,
      },
      {
        optimistic: false,
        failure: '今は状況を参考にできません。データ設定を確認してください。',
      },
    );
    if (saved) setConsentOpen(false);
  };

  const declineCollection = async () => {
    if (!settings) return;
    const saved = await persist(
      {
        ...settings,
        consent: 'declined',
        consent_version: ACTIVITY_COLLECTION_CONSENT_VERSION,
        collection_enabled: false,
      },
      { optimistic: false },
    );
    if (saved) setConsentOpen(false);
  };

  if (loading) {
    return (
      <section className="conversation-settings-panel" aria-label="会話設定" aria-busy="true">
        <p role="status">会話設定を読み込み中…</p>
      </section>
    );
  }

  if (!settings) {
    return (
      <section className="conversation-settings-panel" aria-label="会話設定">
        {feedback ? <p role="alert">{feedback.text}</p> : null}
      </section>
    );
  }

  const selectedFrequency = frequencyIndex(settings.frequency);
  const frequencyPosition = selectedFrequency >= 0
    ? selectedFrequency
    : closestFrequencyIndex(settings.frequency);
  const frequencySentence = selectedFrequency >= 0
    ? FREQUENCY_PRESETS[selectedFrequency]!.sentence
    : customFrequencySentence(settings.frequency);
  const noTriggers = !settings.triggers.return_after_enabled
    && !settings.triggers.long_session_enabled
    && !settings.triggers.category_change_enabled;
  const snoozeActive = settings.proactive_snoozed_until !== null
    && settings.proactive_snoozed_until > Math.floor(Date.now() / 1000);
  const collectionActive = settings.collection_enabled
    && settings.consent === 'accepted'
    && settings.consent_version === ACTIVITY_COLLECTION_CONSENT_VERSION;

  return (
    <section
      className="conversation-settings-panel"
      aria-label="会話設定"
      aria-busy={saving}
    >
      <header className="conversation-settings-panel__heading">
        <div>
          <p className="eyebrow">Conversation</p>
          <h1>会話</h1>
        </div>
        <span>全キャラクター共通</span>
      </header>

      {feedback?.kind === 'error' ? <p className="conversation-feedback" role="alert">{feedback.text}</p> : null}
      {feedback?.kind === 'status' ? <p className="conversation-feedback" role="status">{feedback.text}</p> : null}

      <SettingsSection id="conversation-runtime-title" title="現在の動作状況">
        <div className="conversation-runtime-status" aria-live="polite">
          <div>
            <strong>
              {activeMode?.mode === 'focus'
                ? '集中モード'
                : activeMode?.mode === 'night'
                  ? '夜間モード'
                  : '通常モード'}
            </strong>
            <p>
              {collectionHealth?.status === 'healthy'
                ? '作業状況を安全に収集中です'
                : collectionHealth?.status === 'degraded'
                  ? '収集状態を確認できません'
                  : '収集は停止中です'}
            </p>
          </div>
          <label>
            <span>動作モード</span>
            <select
              aria-label="動作モード"
              disabled={saving}
              value={settings.manual_mode_override ?? 'automatic'}
              onChange={(event) => {
                const value = event.target.value;
                void persist({
                  ...settings,
                  manual_mode_override: value === 'automatic'
                    ? null
                    : value as BehaviorSettingsDto['manual_mode_override'],
                });
              }}
            >
              <option value="automatic">自動</option>
              <option value="normal">通常</option>
              <option value="focus">集中</option>
              <option value="night">夜間</option>
            </select>
          </label>
        </div>
        <div className="conversation-activity-review">
          {activityPage?.sessions?.length ? (
            <ul aria-label="最近のアクティビティ">
              {activityPage.sessions.map((session) => (
                <li key={session.id}>
                  <span>{session.category}</span>
                  <strong>{session.display_app}</strong>
                  <small>{session.display_title}</small>
                </li>
              ))}
            </ul>
          ) : (
            <p className="conversation-settings-note">
              表示できるアクティビティはまだありません。
            </p>
          )}
        </div>
      </SettingsSection>

      <SettingsSection id="conversation-proactive-title" title="向こうから話しかけてもらう">
        <div className="conversation-setting-row">
          <div>
            <strong>自発的な会話</strong>
            <p>あなたから話しかけていないときも、状況に合う一言を届けます。</p>
          </div>
          <Switch
            label="向こうから話しかけてもらう"
            checked={settings.proactive_master_enabled}
            disabled={saving}
            describedBy={!settings.proactive_master_enabled ? 'proactive-master-help' : undefined}
            onChange={(checked) => {
              void persist({ ...settings, proactive_master_enabled: checked });
            }}
          />
        </div>
        {!settings.proactive_master_enabled ? (
          <p id="proactive-master-help" className="conversation-settings-note">
            現在は向こうから話しかけません。設定はONにしたときから使われます。
          </p>
        ) : null}
      </SettingsSection>

      <SettingsSection id="conversation-frequency-title" title="話しかける頻度">
        <div className="conversation-frequency">
          <div className="conversation-frequency__labels" aria-hidden="true">
            <span>かなり控えめ</span>
            <span>頻繁</span>
          </div>
          <input
            type="range"
            aria-label="話しかける頻度"
            min="0"
            max="4"
            step="1"
            value={frequencyPosition}
            disabled={saving}
            onChange={(event) => {
              const preset = FREQUENCY_PRESETS[Number(event.target.value)];
              if (preset) void persist({ ...settings, frequency: preset.policy });
            }}
          />
          <p className="conversation-frequency__value">
            {selectedFrequency < 0 ? <span>以前の設定</span> : <strong>{FREQUENCY_PRESETS[selectedFrequency]!.label}</strong>}
            {frequencySentence}
          </p>
          <p className="conversation-settings-note">実際の回数は、会話の流れや状況によって少なくなることがあります。</p>
        </div>
      </SettingsSection>

      <SettingsSection id="conversation-triggers-title" title="話しかけるきっかけ">
        <div className="conversation-trigger-list">
          <div className="conversation-trigger-row">
            <div>
              <strong>戻ってきたとき</strong>
              <p>席を離れてから戻った流れを見ます。</p>
            </div>
            <Switch
              label="戻ってきたとき"
              checked={settings.triggers.return_after_enabled}
              disabled={saving}
              onChange={(checked) => updateTriggers({ return_after_enabled: checked })}
            />
            {settings.triggers.return_after_enabled ? (
              <label>
                <span>離れてから</span>
                <select
                  aria-label="離れてから"
                  value={settings.triggers.return_after_minutes}
                  disabled={saving}
                  onChange={(event) => updateTriggers({
                    return_after_minutes: Number(event.target.value),
                  })}
                >
                  {selectOptions(settings.triggers.return_after_minutes, RETURN_OPTIONS, '分以上')}
                </select>
              </label>
            ) : null}
          </div>

          <div className="conversation-trigger-row">
            <div>
              <strong>長く作業しているとき</strong>
              <p>同じ作業が長く続いた流れを見ます。</p>
            </div>
            <Switch
              label="長く作業しているとき"
              checked={settings.triggers.long_session_enabled}
              disabled={saving}
              onChange={(checked) => updateTriggers({ long_session_enabled: checked })}
            />
            {settings.triggers.long_session_enabled ? (
              <label>
                <span>続けて</span>
                <select
                  aria-label="続けて"
                  value={settings.triggers.long_session_minutes}
                  disabled={saving}
                  onChange={(event) => updateTriggers({
                    long_session_minutes: Number(event.target.value),
                  })}
                >
                  {selectOptions(settings.triggers.long_session_minutes, LONG_SESSION_OPTIONS)}
                </select>
              </label>
            ) : null}
          </div>

          <div className="conversation-trigger-row">
            <div>
              <strong>作業内容が変わったとき</strong>
              <p>前面のアプリや画面の題名が変わった流れを見ます。</p>
            </div>
            <Switch
              label="作業内容が変わったとき"
              checked={settings.triggers.category_change_enabled}
              disabled={saving}
              onChange={(checked) => updateTriggers({ category_change_enabled: checked })}
            />
            {settings.triggers.category_change_enabled ? (
              <label>
                <span>変化が続いて</span>
                <select
                  aria-label="変化が続いて"
                  value={settings.triggers.category_change_minutes}
                  disabled={saving}
                  onChange={(event) => updateTriggers({
                    category_change_minutes: Number(event.target.value),
                  })}
                >
                  {selectOptions(settings.triggers.category_change_minutes, CATEGORY_CHANGE_OPTIONS)}
                </select>
              </label>
            ) : null}
          </div>
        </div>
        {noTriggers ? <p className="conversation-settings-note">話しかけるきっかけがありません。</p> : null}
        {!collectionActive ? (
          <p className="conversation-settings-note">
            作業中の状況を参考にしないため、現在は使われません。
          </p>
        ) : null}
      </SettingsSection>

      <SettingsSection id="conversation-quiet-hours-title" title="話しかけない時間帯">
        <p className="conversation-settings-intro">
          曜日と時間を指定すると、その間は向こうからの話しかけだけを静かに止めます。
        </p>
        <div className="quiet-hours-list">
          {settings.quiet_hours.map((rule, index) => (
            <fieldset
              key={rule.rule_id}
              className="quiet-hours-row"
              aria-label={`話しかけない時間帯 ${index + 1}`}
            >
              <legend className="sr-only">話しかけない時間帯 {index + 1}</legend>
              <div className="quiet-hours-days" aria-label="曜日">
                {DAYS.map((day) => (
                  <label key={day.value}>
                    <input
                      type="checkbox"
                      aria-label={day.name}
                      checked={rule.days_of_week.includes(day.value)}
                      disabled={saving}
                      onChange={(event) => {
                        const days = event.target.checked
                          ? [...rule.days_of_week, day.value].sort()
                          : rule.days_of_week.filter((value) => value !== day.value);
                        updateQuietHours(index, { days_of_week: days });
                      }}
                    />
                    <span>{day.short}</span>
                  </label>
                ))}
              </div>
              <div className="quiet-hours-times">
                <label>
                  <span className="sr-only">開始時刻</span>
                  <input
                    type="time"
                    aria-label="開始時刻"
                    value={rule.start_local_time}
                    disabled={saving}
                    onChange={(event) => updateQuietHours(index, {
                      start_local_time: event.target.value,
                    })}
                  />
                </label>
                <span aria-hidden="true">―</span>
                <label>
                  <span className="sr-only">終了時刻</span>
                  <input
                    type="time"
                    aria-label="終了時刻"
                    value={rule.end_local_time}
                    disabled={saving}
                    onChange={(event) => updateQuietHours(index, {
                      end_local_time: event.target.value,
                    })}
                  />
                </label>
              </div>
              <Switch
                label="この時間帯を使う"
                checked={rule.enabled}
                disabled={saving}
                onChange={(checked) => updateQuietHours(index, { enabled: checked })}
              />
              <button
                type="button"
                className="conversation-text-button"
                disabled={saving}
                onClick={() => void deleteQuietHours(index)}
              >
                削除
              </button>
            </fieldset>
          ))}
        </div>
        {quietHoursError ? <p className="conversation-settings-error" role="alert">{quietHoursError}</p> : null}
        {undoQuietHours ? (
          <p className="quiet-hours-undo" role="status">
            時間帯を削除しました。
            <button type="button" disabled={saving} onClick={() => void restoreQuietHours()}>
              元に戻す
            </button>
          </p>
        ) : null}
        <button
          type="button"
          className="conversation-add-button"
          disabled={saving || settings.quiet_hours.length >= MAX_QUIET_HOURS_RULES}
          onClick={addQuietHours}
        >
          時間帯を追加
        </button>
        {settings.quiet_hours.length >= MAX_QUIET_HOURS_RULES ? (
          <p className="conversation-settings-note">時間帯は32件まで追加できます。</p>
        ) : null}
      </SettingsSection>

      <SettingsSection id="conversation-snooze-title" title="しばらく静かにしてもらう">
        <div className="conversation-setting-row">
          <div>
            <strong>一時的に話しかけない</strong>
            <p>主スイッチや頻度は変えず、決めた時間までだけ静かにします。</p>
          </div>
          <button
            type="button"
            className="secondary-button"
            disabled={saving}
            onClick={() => setSnoozeChoicesOpen((current) => !current)}
          >
            時間を選ぶ
          </button>
        </div>
        {snoozeChoicesOpen ? (
          <div className="snooze-choices" aria-label="静かにする期間">
            <button type="button" disabled={saving} onClick={() => setSnooze(Math.floor(Date.now() / 1000) + 60 * 60)}>1時間</button>
            <button type="button" disabled={saving} onClick={() => setSnooze(Math.floor(Date.now() / 1000) + 3 * 60 * 60)}>3時間</button>
            <button type="button" disabled={saving} onClick={() => setSnooze(nextLocalMidnightSeconds())}>今日いっぱい</button>
          </div>
        ) : null}
        {snoozeActive && settings.proactive_snoozed_until !== null ? (
          <div className="snooze-status" role="status">
            <span>{formatSnooze(settings.proactive_snoozed_until)}</span>
            <button
              type="button"
              className="conversation-text-button"
              disabled={saving}
              onClick={() => setSnooze(null)}
            >
              また話しかけてもらう
            </button>
          </div>
        ) : null}
      </SettingsSection>

      <SettingsSection id="conversation-context-title" title="作業中の状況を参考にしてもらう">
        <div className="conversation-setting-row">
          <div>
            <strong>この端末での作業状況</strong>
            <p>開いているアプリや画面の題名から、戻ってきたときや作業の変化を判断します。</p>
          </div>
          <Switch
            label="作業中の状況を参考にしてもらう"
            checked={collectionActive}
            disabled={saving}
            onChange={requestCollection}
          />
        </div>
        {consentOpen ? (
          <div className="collection-consent">
            <strong>参考にするもの</strong>
            <ul>
              <li>前面にあるアプリ</li>
              <li>画面の題名</li>
            </ul>
            <p>
              情報はこの端末内へ暗号化して保存し、初期設定では30日後に削除します。
              保存期間や参考にしないアプリは「設定 &gt; データ」で変更できます。
            </p>
            <div>
              <button type="button" disabled={saving} onClick={() => void acceptCollection()}>
                同意して有効にする
              </button>
              <button
                type="button"
                className="secondary-button"
                disabled={saving}
                onClick={() => void declineCollection()}
              >
                今は使わない
              </button>
            </div>
          </div>
        ) : null}
        {!collectionActive && !consentOpen ? (
          <p className="conversation-settings-note">
            状況参照はOFFです。会話履歴や長期記憶には影響しません。
          </p>
        ) : null}
        <p className="conversation-settings-note">
          保存期間、除外、保存済みデータの確認と削除は「設定 &gt; データ」で行えます。
        </p>
      </SettingsSection>
    </section>
  );
}
