import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveLive2DDevAsset } from './live2d-dev-plugin';

describe('Live2D dev asset resolver', () => {
  it('serves only canonical regular files for GET and HEAD', () => {
    const root = mkdtempSync(join(tmpdir(), 'pw-live2d-'));
    mkdirSync(join(root, 'models')); writeFileSync(join(root, 'models', 'a.json'), '{}');
    expect(resolveLive2DDevAsset(root, '/__live2d_dev__/models/a.json')).toMatchObject({ status: 200 });
    expect(resolveLive2DDevAsset(root, '/__live2d_dev__/models/a.json', 'HEAD')).toMatchObject({ status: 200 });
    expect(resolveLive2DDevAsset(root, '/__live2d_dev__/models/a.json', 'POST')).toEqual({ status: 405 });
  });
  it.each(['../x','%2e%2e/x','%252e%252e/x','bad%ZZ','a%5cb','a%00b'])('rejects unsafe path %s', value => {
    const root = mkdtempSync(join(tmpdir(), 'pw-live2d-'));
    expect(resolveLive2DDevAsset(root, `/__live2d_dev__/${value}`).status).not.toBe(200);
  });
});
