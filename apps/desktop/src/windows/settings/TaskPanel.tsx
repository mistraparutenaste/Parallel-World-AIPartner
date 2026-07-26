import type { CommitmentSummaryDto, MemoryCenterDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export function TaskPanel() {
  const [tasks, setTasks] = useState<CommitmentSummaryDto[]>([]);
  const [content, setContent] = useState('');
  const [editing, setEditing] = useState<CommitmentSummaryDto | null>(null);
  const [editContent, setEditContent] = useState('');
  const [error, setError] = useState<string | null>(null);

  const accept = (center: MemoryCenterDto) => {
    setTasks(center.commitments);
    setError(null);
  };
  useEffect(() => {
    void invoke<MemoryCenterDto>('get_memory_center').then(accept).catch(() => {
      setError('タスクを読み込めませんでした。');
    });
  }, []);

  const create = async () => {
    if (!content.trim()) return;
    try {
      accept(await invoke<MemoryCenterDto>('create_commitment', {
        content: content.trim(),
        dueAt: null,
      }));
      setContent('');
    } catch {
      setError('タスクを追加できませんでした。');
    }
  };

  const update = async (task: CommitmentSummaryDto, nextContent: string, status: string) => {
    try {
      accept(await invoke<MemoryCenterDto>('update_commitment', {
        id: task.id,
        content: nextContent.trim(),
        status,
        dueAt: task.due_at,
        expectedRevision: task.revision,
      }));
      setEditing(null);
    } catch (cause) {
      setError(String(cause).includes('COMMITMENT_CONFLICT')
        ? '別の画面でタスクが更新されました。再読み込みしてください。'
        : 'タスクを更新できませんでした。');
    }
  };

  const remove = async (task: CommitmentSummaryDto) => {
    if (!window.confirm('このタスクを削除しますか？')) return;
    try {
      accept(await invoke<MemoryCenterDto>('delete_commitment', { id: task.id }));
    } catch {
      setError('タスクを削除できませんでした。');
    }
  };

  return (
    <section aria-labelledby="tasks-heading">
      <h2 id="tasks-heading">タスク</h2>
      <p>あなたとAIの両方が参照できる共有メモです。</p>
      {error ? <p role="alert">{error}</p> : null}
      <div className="data-export-row">
        <label>
          <span>新しいタスク</span>
          <input value={content} onChange={(event) => setContent(event.target.value)} />
        </label>
        <button type="button" disabled={!content.trim()} onClick={() => void create()}>追加</button>
      </div>
      {tasks.length === 0 ? <p>進行中のタスクはありません。</p> : (
        <ul>
          {tasks.map((task) => (
            <li key={task.id}>
              {editing?.id === task.id ? (
                <>
                  <input
                    aria-label={`タスク ${task.id} の内容`}
                    value={editContent}
                    onChange={(event) => setEditContent(event.target.value)}
                  />
                  <button type="button" disabled={!editContent.trim()} onClick={() => void update(task, editContent, 'open')}>保存</button>
                  <button type="button" onClick={() => setEditing(null)}>キャンセル</button>
                </>
              ) : (
                <>
                  <span>{task.content}</span>
                  <button type="button" onClick={() => void update(task, task.content, 'completed')}>完了</button>
                  <button type="button" onClick={() => { setEditing(task); setEditContent(task.content); }}>編集</button>
                  <button type="button" onClick={() => void remove(task)}>削除</button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
