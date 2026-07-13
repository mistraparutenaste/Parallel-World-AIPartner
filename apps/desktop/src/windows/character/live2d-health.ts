import { invoke } from '@tauri-apps/api/core';

export type Live2DFailureCode =
  | 'core_missing'
  | 'renderer_initialization_failed'
  | 'model_load_failed';

export function reportLive2DFailure(code: Live2DFailureCode): Promise<void> {
  return invoke('report_runtime_failure', { feature: 'live2d', code });
}

export function createLive2DFailureReporter(
  report: (code: Live2DFailureCode) => unknown = reportLive2DFailure,
): (code: Live2DFailureCode) => void {
  let reported = false;
  return (code) => {
    if (reported) return;
    reported = true;
    void report(code);
  };
}

export function reportLive2DSuccess(): Promise<void> {
  return invoke('report_runtime_success', { feature: 'live2d' });
}

export function rearmLive2D(): Promise<void> {
  return invoke('rearm_runtime_feature', { feature: 'live2d' });
}

export function retryLive2D(): Promise<void> {
  return rearmLive2D();
}

export function handleLive2DRetry(reboot: () => void): void {
  reboot();
}
