import { invoke } from '@tauri-apps/api/core';

let installed = false;

function submit(kind: 'window_error' | 'unhandled_rejection', line?: number, column?: number) {
  void invoke('report_frontend_error', {
    report: { schema_version: 1, kind, line: line ?? null, column: column ?? null },
  }).catch(() => undefined);
}

/** Installs bounded metadata-only handlers. Raw messages and stacks never cross IPC. */
export function installFrontendDiagnostics() {
  if (installed) return;
  installed = true;
  window.addEventListener('error', (event) => {
    submit('window_error', event.lineno, event.colno);
  });
  window.addEventListener('unhandledrejection', (event) => {
    submit('unhandled_rejection');
  });
}
