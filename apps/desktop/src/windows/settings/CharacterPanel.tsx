import type { CharacterManifestDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useState } from 'react';

/**
 * Character section of the settings window: loads the active model's
 * manifest and lets the user switch expressions and play motions.
 * All requests go through validated Rust commands.
 */
export function CharacterPanel() {
  const [manifest, setManifest] = useState<CharacterManifestDto | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const loadManifest = useCallback(() => {
    let cancelled = false;
    invoke<CharacterManifestDto>('get_character_manifest')
      .then((loaded) => {
        if (!cancelled) {
          setManifest(loaded);
          setMessage(null);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setMessage(`キャラクターモデルを読み込めません: ${String(error)}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => loadManifest(), [loadManifest]);

  const applyExpression = (name: string) => {
    if (name === '') {
      return;
    }
    invoke('set_expression', { name }).catch((error: unknown) => {
      setMessage(`表情を適用できません: ${String(error)}`);
    });
  };

  const playMotion = (group: string) => {
    invoke('start_motion', { group }).catch((error: unknown) => {
      setMessage(`モーションを再生できません: ${String(error)}`);
    });
  };

  return (
    <section aria-label="キャラクター設定">
      <h2>キャラクター</h2>
      {message !== null && <p role="alert">{message}</p>}
      {manifest === null && (
        <button type="button" onClick={() => loadManifest()}>
          再読み込み
        </button>
      )}
      {manifest !== null && (
        <>
          <div>
            <label htmlFor="expression-select">表情</label>
            <select
              id="expression-select"
              defaultValue=""
              onChange={(event) => applyExpression(event.target.value)}
            >
              <option value="" disabled>
                表情を選択
              </option>
              {manifest.expressions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </div>
          <div>
            <h3>モーション</h3>
            <ul>
              {manifest.motion_groups.map((group) => (
                <li key={group.name}>
                  <button type="button" onClick={() => playMotion(group.name)}>
                    {group.name} を再生
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </>
      )}
    </section>
  );
}
