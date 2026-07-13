import type { TechnicalLogChunkDto, TechnicalLogCursorDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useMemo, useState } from 'react';

const MAX_LINES = 2_000;

export function TechnicalLogPanel() {
  const [lines, setLines] = useState<string[]>([]);
  const [filter, setFilter] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    let cursor: TechnicalLogCursorDto | null = null;
    let reading = false;
    const refresh = async () => {
      if (reading) return;
      reading = true;
      try {
        for (let chunkIndex = 0; chunkIndex < 4 && mounted; chunkIndex += 1) {
          const chunk = await invoke<TechnicalLogChunkDto>('read_technical_log', { cursor });
          if (!mounted) return;
          setLines((current) => {
            const next = chunk.reset ? chunk.lines : [...current, ...chunk.lines];
            return next.slice(-MAX_LINES);
          });
          cursor = chunk.next_cursor;
          setError(null);
          if (!chunk.has_more) break;
        }
      } catch (problem) {
        if (mounted) setError(String(problem));
      } finally {
        reading = false;
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 2_000);
    return () => {
      mounted = false;
      window.clearInterval(interval);
    };
  }, []);

  const visibleLines = useMemo(() => {
    const wanted = filter.trim().toLocaleLowerCase();
    return wanted === ''
      ? lines
      : lines.filter((line) => line.toLocaleLowerCase().includes(wanted));
  }, [filter, lines]);

  return (
    <section className="log-panel" aria-label="技術ログ">
      <div className="log-toolbar">
        <label>
          <span className="sr-only">技術ログを絞り込み</span>
          <input value={filter} placeholder="現在のログを絞り込み" onChange={(event) => setFilter(event.target.value)} />
        </label>
        <span className="log-count">{visibleLines.length.toLocaleString('ja-JP')} 行</span>
      </div>
      {error ? <p role="alert">{error}</p> : null}
      {lines.length === 0 && !error ? <p className="empty-state">技術ログはまだありません。</p> : null}
      <pre className="technical-log" tabIndex={0}>
        {visibleLines.join('\n')}
      </pre>
    </section>
  );
}
