import { invoke } from '@tauri-apps/api/core';
import { beforeEach, expect, test, vi } from 'vitest';
import { createLive2DFailureReporter, handleLive2DRetry, reportLive2DFailure, reportLive2DSuccess, rearmLive2D, retryLive2D } from './live2d-health';

test('retry waits for successful rearm before starting boot', async () => {
  const invoke = vi.mocked(await import('@tauri-apps/api/core')).invoke;
  let release!: () => void;
  invoke.mockReturnValueOnce(new Promise<void>((resolve) => { release = resolve; }));
  let booted = false;
  const retry = rearmLive2D().then(() => { booted = true; });
  await Promise.resolve();
  expect(booted).toBe(false);
  release();
  await retry;
  expect(booted).toBe(true);
});

test('failed rearm leaves the Live2D fallback active', async () => {
  const invoke = vi.mocked(await import('@tauri-apps/api/core')).invoke;
  invoke.mockRejectedValueOnce(new Error('circuit is not open'));
  await expect(retryLive2D()).rejects.toThrow('circuit is not open');
});

test('retry event starts a new renderer boot', () => {
  const reboot = vi.fn();
  handleLive2DRetry(reboot);
  expect(reboot).toHaveBeenCalledOnce();
});

test('controller callback and catch report one failure per boot', () => {
  const report = vi.fn();
  const reportOnce = createLive2DFailureReporter(report);
  reportOnce('renderer_initialization_failed');
  reportOnce('renderer_initialization_failed');
  expect(report).toHaveBeenCalledTimes(1);
});

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
