import { invoke } from '@tauri-apps/api/core';
import { beforeEach, expect, test, vi } from 'vitest';
import { reportLive2DFailure, reportLive2DSuccess, rearmLive2D } from './live2d-health';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));

beforeEach(() => vi.mocked(invoke).mockClear());

test('reports a typed Live2D failure to the Rust supervisor', async () => {
  await reportLive2DFailure('model_load_failed');
  expect(invoke).toHaveBeenCalledWith('report_runtime_failure', {
    feature: 'live2d', code: 'model_load_failed',
  });
});

test('reports successful Live2D boot through the Rust supervisor', async () => {
  await reportLive2DSuccess();
  expect(invoke).toHaveBeenCalledWith('report_runtime_success', { feature: 'live2d' });
});

test('rearms Live2D before retrying its renderer', async () => {
  await rearmLive2D();
  expect(invoke).toHaveBeenCalledWith('rearm_runtime_feature', { feature: 'live2d' });
});
