import type { LlmSettingsDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

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
          setSettings(loaded);
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
