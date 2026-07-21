import type { LlmSettingsDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

type Provider = LlmSettingsDto['provider'];

const PROVIDER_PRESETS: Record<
  Provider,
  { label: string; baseUrl?: string; remote: boolean }
> = {
  local: { label: 'ローカル / LAN', remote: false },
  openai: {
    label: 'OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    remote: true,
  },
  gemini: {
    label: 'Google Gemini（OpenAI互換）',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai',
    remote: true,
  },
  opencode_zen: {
    label: 'OpenCode Zen（Chat Completions）',
    baseUrl: 'https://opencode.ai/zen/v1',
    remote: true,
  },
  custom: { label: 'カスタム（OpenAI互換）', remote: true },
};

/**
 * LLM section of the settings window: endpoint, model and prompt
 * configuration. Non-loopback endpoints require the explicit
 * allow-remote opt-in and are validated on save by the Rust side.
 */
export function LlmPanel() {
  const [settings, setSettings] = useState<LlmSettingsDto | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<LlmSettingsDto>('get_llm_settings')
      .then((loaded) => {
        if (!cancelled) {
          setSettings({
            ...loaded,
            api_key: loaded.api_key ?? '',
            clear_api_key: loaded.clear_api_key ?? false,
          });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setMessage(`LLM設定を読み込めません: ${String(error)}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const update = (patch: Partial<LlmSettingsDto>) => {
    setSettings((current) => (current ? { ...current, ...patch } : current));
  };

  const selectProvider = (provider: Provider) => {
    const preset = PROVIDER_PRESETS[provider];
    update({
      provider,
      ...(preset.baseUrl ? { base_url: preset.baseUrl } : {}),
      allow_remote: preset.remote,
      api_key: '',
      api_key_configured: false,
      clear_api_key: false,
    });
  };

  const save = () => {
    if (!settings) {
      return;
    }
    setMessage(null);
    invoke('set_llm_settings', { settings })
      .then(() => {
        setMessage('保存しました。次のメッセージから適用されます。');
      })
      .catch((error: unknown) => {
        setMessage(`保存できません: ${String(error)}`);
      });
  };

  return (
    <section aria-label="LLM設定">
      <h2>LLM</h2>
      {message !== null && <p role="status">{message}</p>}
      {settings !== null && (
        <>
          <div>
            <label htmlFor="llm-provider">プロバイダー</label>
            <select
              id="llm-provider"
              value={settings.provider}
              onChange={(event) => selectProvider(event.target.value as Provider)}
            >
              {Object.entries(PROVIDER_PRESETS).map(([value, preset]) => (
                <option key={value} value={value}>
                  {preset.label}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label htmlFor="llm-base-url">接続先 (OpenAI互換)</label>
            <input
              id="llm-base-url"
              type="text"
              value={settings.base_url}
              onChange={(event) => update({ base_url: event.target.value })}
            />
          </div>
          <div>
            <label htmlFor="llm-model">モデル名</label>
            <input
              id="llm-model"
              type="text"
              value={settings.model}
              onChange={(event) => update({ model: event.target.value })}
            />
          </div>
          {settings.provider !== 'local' && (
            <div>
              <label htmlFor="llm-api-key">APIキー</label>
              <input
                id="llm-api-key"
                type="password"
                value={settings.api_key}
                placeholder={
                  settings.api_key_configured
                    ? '保存済み（変更する場合のみ入力）'
                    : 'APIキーを入力'
                }
                autoComplete="off"
                onChange={(event) =>
                  update({ api_key: event.target.value, clear_api_key: false })
                }
              />
              {settings.api_key_configured && (
                <button
                  type="button"
                  onClick={() => update({ api_key: '', clear_api_key: true })}
                >
                  保存済みAPIキーを削除
                </button>
              )}
              <p>
                APIキーはOSの資格情報ストアに保存され、設定ファイルや画面には返されません。
              </p>
            </div>
          )}
          {settings.provider === 'opencode_zen' && (
            <p>
              OpenCode ZenはChat Completions対応モデルのみ利用できます。Responses
              API専用モデルは未対応です。
            </p>
          )}
          <div>
            <label>
              <input
                type="checkbox"
                checked={settings.allow_remote}
                onChange={(event) =>
                  update({ allow_remote: event.target.checked })
                }
              />
              ループバック以外への接続を許可
            </label>
          </div>
          <div>
            <label>
              <input
                type="checkbox"
                checked={settings.strip_emoji}
                onChange={(event) =>
                  update({ strip_emoji: event.target.checked })
                }
              />
              応答から絵文字を除去する
            </label>
          </div>
          <div>
            <label htmlFor="llm-character-prompt">キャラクター設定</label>
            <textarea
              id="llm-character-prompt"
              rows={3}
              value={settings.character_prompt}
              onChange={(event) =>
                update({ character_prompt: event.target.value })
              }
            />
          </div>
          <button type="button" onClick={save}>
            保存
          </button>
        </>
      )}
    </section>
  );
}
