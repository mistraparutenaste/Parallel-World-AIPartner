import type { UpdateStateDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

const ACTIVE = new Set<UpdateStateDto['status']>(['checking', 'downloading', 'installing']);

export function UpdatesPanel() {
  const [state, setState] = useState<UpdateStateDto | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void invoke<UpdateStateDto>('get_update_state').then((value) => {
      if (active) setState(value);
    }).catch((error) => { if (active) setRequestError(String(error)); });
    let dispose: (() => void) | undefined;
    void Promise.resolve()
      .then(() => listen<UpdateStateDto>('update-progress', ({ payload }) => {
        if (active) setState(payload);
      }))
      .then((unlisten) => { if (active) dispose = unlisten; else unlisten(); })
      .catch(() => undefined);
    return () => { active = false; dispose?.(); };
  }, []);

  const busy = state !== null && ACTIVE.has(state.status);
  const check = () => {
    setRequestError(null);
    void invoke<UpdateStateDto>('check_for_updates').then(setState).catch((error) => setRequestError(String(error)));
  };
  const install = () => {
    if (!state?.available_version) return;
    const approvedVersion = state.available_version;
    setRequestError(null);
    void invoke('install_update', { approvedVersion }).catch((error) => setRequestError(String(error)));
  };

  return (
    <section aria-labelledby="updates-heading">
      <h2 id="updates-heading">アプリの更新</h2>
      {state ? <p>現在のバージョン: {state.current_version}</p> : <p>更新状態を読み込み中</p>}
      {state?.status === 'disabled' ? <p>このビルドでは自動更新が無効です。</p> : null}
      {state?.available_version ? (
        <>
          <p>利用可能なバージョン: {state.available_version}</p>
          {state.notes ? <p>{state.notes}</p> : null}
          <button type="button" disabled={busy} onClick={install}>
            {state.available_version} をインストール
          </button>
        </>
      ) : null}
      <button type="button" disabled={busy || state?.status === 'disabled'} onClick={check}>更新を確認</button>
      {state?.status === 'restart_pending' ? <p>更新を適用しました。再起動します。</p> : null}
      {state?.error || requestError ? <p role="alert">{state?.error ?? requestError}</p> : null}
    </section>
  );
}
