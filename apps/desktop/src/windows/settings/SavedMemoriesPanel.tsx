import type { MemoryCenterDto, MemorySummaryDto } from '@parallel-world/contracts';
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

export function SavedMemoriesPanel() {
  const [center, setCenter] = useState<MemoryCenterDto | null>(null);
  const [editing, setEditing] = useState<MemorySummaryDto | null>(null);
  const [content, setContent] = useState('');
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    try {
      setCenter(await invoke<MemoryCenterDto>('get_memory_center'));
      setError(null);
    } catch {
      setError('保存済みのメモリーを読み込めませんでした。');
    }
  };
  useEffect(() => { void load(); }, []);

  const beginEdit = async (memory: MemorySummaryDto) => {
    try {
      setContent(await invoke<string>('get_memory_content', { memoryId: memory.id }));
      setEditing(memory);
    } catch {
      setError('メモリーの内容を読み込めませんでした。');
    }
  };
  const save = async () => {
    if (!editing || !content.trim()) return;
    try {
      setCenter(await invoke<MemoryCenterDto>('update_memory', {
        memoryId: editing.id,
        content: content.trim(),
        expectedRevision: editing.revision,
      }));
      setEditing(null);
      setError(null);
    } catch (cause) {
      setError(String(cause).includes('MEMORY_CONFLICT')
        ? '別の画面でメモリーが更新されました。再読み込みしてください。'
        : 'メモリーを保存できませんでした。');
    }
  };
  const remove = async (memory: MemorySummaryDto) => {
    if (!window.confirm('このメモリーを削除しますか？')) return;
    try {
      setCenter(await invoke<MemoryCenterDto>('delete_memory', { memoryId: memory.id }));
    } catch {
      setError('メモリーを削除できませんでした。');
    }
  };

  const setConsent = async (domain: string, consent: string, expectedRevision: number) => {
    try {
      await invoke('set_memory_domain_control', {
        domain,
        consent,
        expectedRevision,
      });
      await load();
    } catch {
      setError('メモリーの保存設定を変更できませんでした。');
    }
  };

  return (
    <section aria-labelledby="saved-memories-heading">
      <h2 id="saved-memories-heading">保存済みのメモリー</h2>
      {error ? <p role="alert">{error}</p> : null}
      {!center ? <p>読み込み中…</p> : center.memories.length === 0 ? (
        <p>保存済みメモリーはありません。</p>
      ) : (
        <table className="memory-table">
          <thead><tr><th>内容</th><th>編集</th><th>削除</th></tr></thead>
          <tbody>{center.memories.map((memory) => (
            <tr key={memory.id}>
              <td>{memory.preview}</td>
              <td><button type="button" onClick={() => void beginEdit(memory)}>編集</button></td>
              <td><button type="button" onClick={() => void remove(memory)}>削除</button></td>
            </tr>
          ))}</tbody>
        </table>
      )}
      {editing ? (
        <div role="group" aria-label="メモリーを編集">
          <label>
            <span>内容</span>
            <textarea maxLength={2000} value={content} onChange={(event) => setContent(event.target.value)} />
          </label>
          <button type="button" disabled={!content.trim()} onClick={() => void save()}>保存</button>
          <button type="button" onClick={() => setEditing(null)}>キャンセル</button>
        </div>
      ) : null}
      {center ? (
        <details>
          <summary>詳細な保存設定</summary>
          {center.domains.map((domain) => (
            <div className="setting-row" key={domain.domain}>
              <strong>{domainLabel[domain.domain] ?? domain.domain}</strong>
              <label>
                <span className="sr-only">
                  {domainLabel[domain.domain] ?? domain.domain}の保存設定
                </span>
                <select
                  value={domain.consent}
                  onChange={(event) => void setConsent(
                    domain.domain,
                    event.target.value,
                    domain.revision,
                  )}
                >
                  <option value="allowed">保存を許可</option>
                  <option value="pending_approval">確認してから保存</option>
                  <option value="never_store">保存しない</option>
                </select>
              </label>
            </div>
          ))}
          <h3>確認待ちの候補</h3>
          {center.pending.length === 0 ? <p>確認待ちの候補はありません。</p> : (
            <ul>{center.pending.map((candidate) => <li key={candidate.id}>{candidate.preview}</li>)}</ul>
          )}
        </details>
      ) : null}
    </section>
  );
}
