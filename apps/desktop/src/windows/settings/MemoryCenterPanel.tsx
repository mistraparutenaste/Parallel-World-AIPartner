import type { MemoryCenterDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

const domainLabel: Record<string, string> = {
  working: '作業中', episode: '出来事', semantic_user: 'ユーザー情報', relationship: '関係性',
  ai_self: 'AIの自己像', procedural: '手順', commitment: '約束', reflection: '振り返り',
};

export function MemoryCenterPanel() {
  const [center, setCenter] = useState<MemoryCenterDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = async () => {
    try { setCenter(await invoke<MemoryCenterDto>('get_memory_center')); setError(null); }
    catch { setError('メモリーセンターを読み込めませんでした。'); }
  };
  useEffect(() => { void load(); }, []);
  const setTemporary = async (temporary: boolean) => {
    if (!center) return;
    try {
      setCenter(await invoke<MemoryCenterDto>('set_temporary_conversation', {
        temporary, expectedRevision: center.temporary_revision,
      }));
      setError(null);
    } catch { setError('一時会話モードを変更できませんでした。'); }
  };
  const setConsent = async (domain: string, consent: string, revision: number) => {
    try {
      await invoke('set_memory_domain_control', { domain, consent, expectedRevision: revision });
      await load();
    } catch { setError('メモリーの保存方針を変更できませんでした。'); }
  };
  return <section className="settings-panel memory-center" aria-label="メモリーセンター">
    <header><h2>メモリーセンター</h2><p>保存方針と確認待ちの候補を管理します。会話本文や秘密情報はここには表示しません。</p></header>
    {error ? <p role="alert">{error}</p> : null}
    {!center ? <p>読み込み中…</p> : <>
      <div className="setting-row">
        <div><strong>一時会話モード</strong><p>この会話から長期メモリー、関係性、約束を保存しません。</p></div>
        <button type="button" role="switch" aria-checked={center.temporary} aria-label="一時会話モード" onClick={() => void setTemporary(!center.temporary)}>{center.temporary ? 'オン' : 'オフ'}</button>
      </div>
      <h3>保存方針</h3>
      <div className="panel-stack">{center.domains.map((control) => <div className="setting-row" key={control.domain}>
        <strong>{domainLabel[control.domain] ?? control.domain}</strong>
        <label><span className="sr-only">{domainLabel[control.domain] ?? control.domain}の保存方針</span><select value={control.consent} onChange={(event) => void setConsent(control.domain, event.target.value, control.revision)}>
          <option value="allowed">保存を許可</option><option value="pending_approval">確認して保存</option><option value="never_store">保存しない</option>
        </select></label>
      </div>)}</div>
      <h3>確認待ち <span aria-label={`${center.pending.length} 件`}>({center.pending.length})</span></h3>
      {center.pending.length === 0 ? <p>確認待ちの候補はありません。</p> : <ul>{center.pending.map((candidate) => <li key={candidate.id}><strong>{domainLabel[candidate.domain] ?? candidate.domain}</strong>: {candidate.preview}</li>)}</ul>}
      <h3>進行中の約束</h3>
      {center.commitments.length === 0 ? <p>進行中の約束はありません。</p> : <ul>{center.commitments.map((commitment) => <li key={commitment.id}>{commitment.status}</li>)}</ul>}
    </>}
  </section>;
}
