import type { MemoryCenterDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

const domainLabel: Record<string, string> = {
  working: '作業中',
  episode: 'エピソード',
  semantic_user: 'ユーザー情報',
  relationship: '関係性',
  ai_self: 'AI自身',
  procedural: '手順・好み',
  commitment: '約束・予定',
  reflection: '振り返り',
};

export function MemoryCenterPanel() {
  const [center, setCenter] = useState<MemoryCenterDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<number | null>(null);

  const load = async () => {
    try {
      setCenter(await invoke<MemoryCenterDto>('get_memory_center'));
      setError(null);
    } catch {
      setError('メモリーセンターを読み込めませんでした。');
    }
  };

  useEffect(() => {
    void load();
  }, []);

  const setTemporary = async (temporary: boolean) => {
    if (!center) return;
    try {
      setCenter(
        await invoke<MemoryCenterDto>('set_temporary_conversation', {
          temporary,
          expectedRevision: center.temporary_revision,
        }),
      );
      setError(null);
    } catch {
      setError('一時会話モードを変更できませんでした。');
    }
  };

  const setConsent = async (domain: string, consent: string, revision: number) => {
    try {
      await invoke('set_memory_domain_control', {
        domain,
        consent,
        expectedRevision: revision,
      });
      await load();
    } catch {
      setError('メモリーの保存設定を変更できませんでした。');
    }
  };

  const deleteMemory = async (memoryId: number) => {
    if (!window.confirm('このメモリーを削除しますか？')) return;
    setDeletingId(memoryId);
    try {
      setCenter(await invoke<MemoryCenterDto>('delete_memory', { memoryId }));
      setError(null);
    } catch {
      setError('メモリーを削除できませんでした。');
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="settings-panel memory-center" aria-label="メモリーセンター">
      <header>
        <h2>メモリーセンター</h2>
        <p>
          保存設定と確認待ちの候補を管理します。会話本文や秘匿情報はここには表示されません。
        </p>
      </header>
      {error ? <p role="alert">{error}</p> : null}
      {!center ? (
        <p>読み込み中…</p>
      ) : (
        <>
          <div className="setting-row">
            <div>
              <strong>一時会話モード</strong>
              <p>この会話から長期メモリー、関係性、約束を保存しません。</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={center.temporary}
              aria-label="一時会話モード"
              onClick={() => void setTemporary(!center.temporary)}
            >
              {center.temporary ? 'オン' : 'オフ'}
            </button>
          </div>

          <h3>保存設定</h3>
          <div className="panel-stack">
            {center.domains.map((control) => (
              <div className="setting-row" key={control.domain}>
                <strong>{domainLabel[control.domain] ?? control.domain}</strong>
                <label>
                  <span className="sr-only">
                    {domainLabel[control.domain] ?? control.domain}の保存設定
                  </span>
                  <select
                    value={control.consent}
                    onChange={(event) =>
                      void setConsent(control.domain, event.target.value, control.revision)
                    }
                  >
                    <option value="allowed">保存を許可</option>
                    <option value="pending_approval">確認してから保存</option>
                    <option value="never_store">保存しない</option>
                  </select>
                </label>
              </div>
            ))}
          </div>

          <h3>
            保存済みメモリー <span aria-label={`${center.memories.length} 件`}>({center.memories.length})</span>
          </h3>
          {center.memories.length === 0 ? (
            <p>保存済みメモリーはありません。</p>
          ) : (
            <ul>
              {center.memories.map((memory) => (
                <li key={memory.id}>
                  <span>
                    {memory.preview}
                    {memory.pinned ? '（固定）' : ''}
                  </span>
                  <button
                    type="button"
                    aria-label={`メモリー ${memory.id} を削除`}
                    disabled={deletingId === memory.id}
                    onClick={() => void deleteMemory(memory.id)}
                  >
                    {deletingId === memory.id ? '削除中…' : '削除'}
                  </button>
                </li>
              ))}
            </ul>
          )}

          <h3>
            確認待ちの候補 <span aria-label={`${center.pending.length} 件`}>({center.pending.length})</span>
          </h3>
          {center.pending.length === 0 ? (
            <p>確認待ちの候補はありません。</p>
          ) : (
            <ul>
              {center.pending.map((candidate) => (
                <li key={candidate.id}>
                  <strong>{domainLabel[candidate.domain] ?? candidate.domain}</strong>：{' '}
                  {candidate.preview}
                </li>
              ))}
            </ul>
          )}

          <h3>進行中の約束</h3>
          {center.commitments.length === 0 ? (
            <p>進行中の約束はありません。</p>
          ) : (
            <ul>
              {center.commitments.map((commitment) => (
                <li key={commitment.id}>{commitment.status}</li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}
