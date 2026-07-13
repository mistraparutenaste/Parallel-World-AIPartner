import type { DiagnosticReportDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

export function DiagnosticsPanel() {
  const [reports, setReports] = useState<DiagnosticReportDto[]>([]);
  const [destination, setDestination] = useState('');
  const [status, setStatus] = useState<string | null>(null);
  useEffect(() => { void invoke<DiagnosticReportDto[]>('list_diagnostic_reports').then(setReports).catch(() => setReports([])); }, []);
  const exportReports = async () => {
    const path = destination.trim(); if (!path) return;
    try { await invoke('export_diagnostic_reports', { destination: path, allowOverwrite: false }); setStatus('エクスポートしました'); }
    catch (error) {
      if (String(error).includes('DESTINATION_EXISTS') && window.confirm('既存ファイルを上書きしますか？')) {
        try {
          await invoke('export_diagnostic_reports', { destination: path, allowOverwrite: true });
          setStatus('エクスポートしました');
        } catch (overwriteError) {
          setStatus(String(overwriteError));
        }
      } else { setStatus(String(error)); }
    }
  };
  return <section aria-labelledby="diagnostics-title"><h2 id="diagnostics-title">診断</h2>
    <p>秘密情報とプロンプト本文を含まない、保持上限付きの診断レポートです。</p>
    <ul>{reports.map((report) => <li key={report.id}>{report.id} — {report.category} — {report.bytes} bytes</li>)}</ul>
    <label htmlFor="diagnostic-export">診断エクスポート先</label>
    <input id="diagnostic-export" value={destination} onChange={(event) => setDestination(event.target.value)} />
    <button type="button" disabled={!destination.trim()} onClick={() => void exportReports()}>診断をエクスポート</button>
    {status ? <p role="status">{status}</p> : null}
  </section>;
}
