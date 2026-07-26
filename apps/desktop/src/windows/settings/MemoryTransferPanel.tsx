import type { DataUsageDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

const format = new Intl.NumberFormat('ja-JP');

export function MemoryTransferPanel() {
  const [csvDestination, setCsvDestination] = useState('');
  const [csvSource, setCsvSource] = useState('');
  const [databaseDestination, setDatabaseDestination] = useState('');
  const [usage, setUsage] = useState<DataUsageDto | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refreshUsage = async () => {
    try {
      setUsage(await invoke<DataUsageDto>('get_data_usage'));
    } catch {
      setError('データ使用量を取得できませんでした。');
    }
  };
  useEffect(() => { void refreshUsage(); }, []);

  const run = async (operation: () => Promise<unknown>, success: string) => {
    setBusy(true);
    setFeedback(null);
    setError(null);
    try {
      await operation();
      setFeedback(success);
      await refreshUsage();
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const exportCsv = () => run(
    async () => {
      try {
        await invoke('export_memories_csv', {
          destination: csvDestination.trim(),
          allowOverwrite: false,
        });
      } catch (cause) {
        if (
          String(cause).includes('DESTINATION_EXISTS')
          && window.confirm('保存先は既に存在します。上書きしますか？')
        ) {
          await invoke('export_memories_csv', {
            destination: csvDestination.trim(),
            allowOverwrite: true,
          });
          return;
        }
        throw cause;
      }
    },
    'メモリーCSVをエクスポートしました。',
  );

  const exportDatabase = () => run(
    async () => {
      try {
        await invoke('export_user_data', {
          destination: databaseDestination.trim(),
          allowOverwrite: false,
        });
      } catch (cause) {
        if (
          String(cause).includes('DESTINATION_EXISTS')
          && window.confirm('保存先は既に存在します。上書きしますか？')
        ) {
          await invoke('export_user_data', {
            destination: databaseDestination.trim(),
            allowOverwrite: true,
          });
          return;
        }
        throw cause;
      }
    },
    'SQLiteバックアップを作成しました。',
  );

  return (
    <section aria-labelledby="memory-transfer-heading" aria-busy={busy}>
      <h2 id="memory-transfer-heading">インポート/エクスポート</h2>
      <p>
        使用量: 会話 {format.format(usage?.conversation_messages ?? 0)}件 /
        長期記憶 {format.format(usage?.long_term_memories ?? 0)}件
      </p>
      <div className="data-export-row">
        <label>
          <span>メモリーCSVの保存先</span>
          <input value={csvDestination} onChange={(event) => setCsvDestination(event.target.value)} />
        </label>
        <button type="button" disabled={busy || !csvDestination.trim()} onClick={() => void exportCsv()}>
          CSVエクスポート
        </button>
      </div>
      <div className="data-export-row">
        <label>
          <span>メモリーCSVの読込元</span>
          <input value={csvSource} onChange={(event) => setCsvSource(event.target.value)} />
        </label>
        <button
          type="button"
          disabled={busy || !csvSource.trim()}
          onClick={() => void run(
            () => invoke('import_memories_csv', { source: csvSource.trim() }),
            'メモリーCSVをインポートしました。',
          )}
        >
          CSVインポート
        </button>
      </div>
      <div className="data-export-row">
        <label>
          <span>SQLiteバックアップ先</span>
          <input value={databaseDestination} onChange={(event) => setDatabaseDestination(event.target.value)} />
        </label>
        <button type="button" disabled={busy || !databaseDestination.trim()} onClick={() => void exportDatabase()}>
          バックアップ
        </button>
      </div>
      {feedback ? <p role="status">{feedback}</p> : null}
      {error ? <p role="alert">{error}</p> : null}
    </section>
  );
}
