import { invoke } from '@tauri-apps/api/core';

export type CharacterRendererFailureCode =
  | 'core_missing'
  | 'renderer_initialization_failed'
  | 'model_load_failed'
  | 'missing_asset'
  | 'invalid_manifest'
  | 'invalid_image'
  | 'selection_required'
  | 'active_character_unavailable'
  | 'transient_asset_read'
  | 'webview_renderer_failed';

const PERMANENT_FAILURES: ReadonlySet<CharacterRendererFailureCode> = new Set([
  'core_missing',
  'missing_asset',
  'invalid_manifest',
  'invalid_image',
  'selection_required',
  'active_character_unavailable',
]);

export function isPermanentCharacterRendererFailure(
  code: CharacterRendererFailureCode,
): boolean {
  return PERMANENT_FAILURES.has(code);
}

export function classifyCharacterRendererFailure(error: unknown): CharacterRendererFailureCode {
  const message = String(error).toLowerCase();
  if (
    message.includes('selection_required')
    || message.includes('selection required')
    || message.includes('selection is required')
  ) {
    return 'selection_required';
  }
  if (
    message.includes('active_character_unavailable')
    || message.includes('active character unavailable')
    || message.includes('active character is unavailable')
  ) {
    return 'active_character_unavailable';
  }
  if (message.includes('core_missing')) return 'core_missing';
  if (message.includes('invalid_image') || message.includes('invalid image')) {
    return 'invalid_image';
  }
  if (
    message.includes('invalid_manifest')
    || message.includes('invalid manifest')
    || message.includes('invalid character profile')
    || message.includes('profile id is duplicated')
    || message.includes('path escapes the characters root')
    || message.includes('invalid character name')
    || message.includes('duplicate static expression')
    || message.includes('default expression is unavailable')
  ) {
    return 'invalid_manifest';
  }
  if (
    message.includes('missing_asset')
    || message.includes('missing asset')
    || message.includes('not found')
    || message.includes('no character profile')
  ) {
    return 'missing_asset';
  }
  return 'renderer_initialization_failed';
}

export function classifyCharacterRendererLoadFailure(
  rendererKind: 'live2d' | 'static_image',
  error: unknown,
): CharacterRendererFailureCode {
  const message = String(error).toLowerCase();
  if (rendererKind === 'static_image') {
    if (message.includes('failed to fetch static expression')) return 'transient_asset_read';
    if (message.includes('canvas context is unavailable')) return 'webview_renderer_failed';
    return 'invalid_image';
  }
  return classifyCharacterRendererFailure(error);
}

export function reportCharacterRendererFailure(
  code: CharacterRendererFailureCode,
): Promise<void> {
  return invoke('report_runtime_failure', { feature: 'character_renderer', code });
}

export function createCharacterRendererFailureReporter(
  report: (code: CharacterRendererFailureCode) => unknown = reportCharacterRendererFailure,
): (code: CharacterRendererFailureCode) => void {
  let reported = false;
  return (code) => {
    if (reported) return;
    reported = true;
    void Promise.resolve(report(code)).catch((error: unknown) => {
      console.error('failed to report character renderer health', error);
    });
  };
}

export function reportCharacterRendererSuccess(): Promise<void> {
  return invoke('report_runtime_success', { feature: 'character_renderer' });
}

export function retryCharacterRenderer(): Promise<void> {
  return invoke('retry_character_renderer');
}
