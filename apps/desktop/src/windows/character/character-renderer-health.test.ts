import { invoke } from '@tauri-apps/api/core';
import { beforeEach, expect, test, vi } from 'vitest';
import {
  classifyCharacterRendererFailure,
  classifyCharacterRendererLoadFailure,
  createCharacterRendererFailureReporter,
  isPermanentCharacterRendererFailure,
  reportCharacterRendererFailure,
  reportCharacterRendererSuccess,
  retryCharacterRenderer,
} from './character-renderer-health';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

beforeEach(() => vi.mocked(invoke).mockClear());

test('reports generic character-renderer health to Rust', async () => {
  await reportCharacterRendererFailure('invalid_image');
  await reportCharacterRendererSuccess();
  await retryCharacterRenderer();

  expect(invoke).toHaveBeenNthCalledWith(1, 'report_runtime_failure', {
    feature: 'character_renderer', code: 'invalid_image',
  });
  expect(invoke).toHaveBeenNthCalledWith(2, 'report_runtime_success', {
    feature: 'character_renderer',
  });
  expect(invoke).toHaveBeenNthCalledWith(3, 'retry_character_renderer');
});

test('deduplicates failure reports for one boot', () => {
  const report = vi.fn();
  const reportOnce = createCharacterRendererFailureReporter(report);
  reportOnce('renderer_initialization_failed');
  reportOnce('invalid_image');
  expect(report).toHaveBeenCalledOnce();
});

test('classifies profile failures as permanent and WebGL startup as transient', () => {
  expect(classifyCharacterRendererFailure(new Error('selection_required'))).toBe('selection_required');
  expect(classifyCharacterRendererFailure(new Error('character selection is required'))).toBe('selection_required');
  expect(classifyCharacterRendererFailure(new Error('active_character_unavailable'))).toBe('active_character_unavailable');
  expect(classifyCharacterRendererFailure(new Error('active character is unavailable: epsilon'))).toBe('active_character_unavailable');
  expect(classifyCharacterRendererFailure(new Error('invalid character profile C:/x: bad field'))).toBe('invalid_manifest');
  expect(classifyCharacterRendererFailure(new Error('no character profile or legacy Live2D model is available'))).toBe('missing_asset');
  expect(classifyCharacterRendererFailure(new Error('core_missing'))).toBe('core_missing');
  expect(classifyCharacterRendererFailure(new Error('WebGL context lost'))).toBe('renderer_initialization_failed');
  expect(isPermanentCharacterRendererFailure('invalid_manifest')).toBe(true);
  expect(isPermanentCharacterRendererFailure('renderer_initialization_failed')).toBe(false);
  expect(classifyCharacterRendererLoadFailure(
    'static_image', new Error('failed to fetch static expression (503): neutral.png'),
  )).toBe('transient_asset_read');
  expect(classifyCharacterRendererLoadFailure(
    'static_image', new Error('decoded expression dimensions do not match'),
  )).toBe('invalid_image');
});
