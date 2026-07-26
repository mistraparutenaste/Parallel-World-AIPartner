import type { MemoryCenterDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export function SecretModePanel() {
  const [center, setCenter] = useState<MemoryCenterDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void invoke<MemoryCenterDto>('get_memory_center').then(setCenter).catch(() => {
      setError('シークレットモードを読み込めませんでした。');
    });
  }, []);

  const toggle = async () => {
    if (!center) return;
    try {
      setCenter(await invoke<MemoryCenterDto>('set_temporary_conversation', {
        temporary: !center.temporary,
        expectedRevision: center.temporary_revision,
      }));
      setError(null);
    } catch {
      setError('シークレットモードを変更できませんでした。');
    }
  };

  return (
    <section aria-labelledby="secret-mode-heading">
      <h2 id="secret-mode-heading">シークレットモード</h2>
      {error ? <p role="alert">{error}</p> : null}
      <div className="setting-row">
        <div>
          <strong>この会話を記憶しない</strong>
          <p>オンの間は、会話から長期メモリー、関係性、約束を保存しません。</p>
        </div>
        <button
          type="button"
          role="switch"
          aria-label="シークレットモード"
          aria-checked={center?.temporary ?? false}
          disabled={!center}
          onClick={() => void toggle()}
        >
          {center?.temporary ? 'オン' : 'オフ'}
        </button>
      </div>
    </section>
  );
}
