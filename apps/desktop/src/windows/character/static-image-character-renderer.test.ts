import type { CharacterRendererDto } from '@parallel-world/contracts';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  StaticImageCharacterRenderer,
  type StaticImageRendererDependencies,
} from './static-image-character-renderer';

type TestBitmap = ImageBitmap & { readonly id: string; readonly alpha: Uint8Array };

const manifest: Extract<CharacterRendererDto, { kind: 'static_image' }> = {
  kind: 'static_image',
  default_expression: 'neutral',
  expressions: [
    { name: 'neutral', image_path: 'C:\\characters\\neutral.png' },
    { name: 'happy', image_path: 'C:\\characters\\happy.png' },
  ],
  width: 2,
  height: 2,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function bitmap(id: string, alpha = new Uint8Array([255, 0, 15, 16]), width = 2, height = 2) {
  return {
    id,
    alpha,
    width,
    height,
    close: vi.fn(),
  } as unknown as TestBitmap;
}

function harness(bitmaps: Record<string, Promise<TestBitmap> | TestBitmap>) {
  const drawImage = vi.fn();
  const clearRect = vi.fn();
  const setTransform = vi.fn();
  const context = { drawImage, clearRect, setTransform } as unknown as CanvasRenderingContext2D;
  const canvas = {
    width: 300,
    height: 200,
    clientWidth: 300,
    clientHeight: 200,
    getContext: vi.fn(() => context),
  } as unknown as HTMLCanvasElement;
  const fetch = vi.fn(async (url: string) => ({
    ok: true,
    status: 200,
    blob: async () => ({ url }) as unknown as Blob,
  }));
  const createImageBitmap = vi.fn(async (blob: Blob) => {
    const url = (blob as unknown as { url: string }).url;
    const value = bitmaps[url];
    if (value === undefined) throw new Error(`missing bitmap ${url}`);
    return await value;
  });
  const createMaskCanvas = vi.fn((_width: number, _height: number) => {
    let current: TestBitmap | null = null;
    return {
      getContext: () => ({
        drawImage: (next: TestBitmap) => { current = next; },
        getImageData: () => {
          const rgba = new Uint8ClampedArray(2 * 2 * 4);
          current?.alpha.forEach((alpha, index) => { rgba[index * 4 + 3] = alpha; });
          return { data: rgba };
        },
      }),
    } as unknown as OffscreenCanvas;
  });
  const deps: StaticImageRendererDependencies = {
    convertFileSrc: (path) => `asset:${path}`,
    fetch,
    createImageBitmap,
    createMaskCanvas,
  };
  return {
    canvas,
    context,
    deps,
    drawImage,
    clearRect,
    setTransform,
    fetch,
    createImageBitmap,
  };
}

describe('StaticImageCharacterRenderer', () => {
  beforeEach(() => vi.restoreAllMocks());

  it('preloads every frame before drawing the default expression', async () => {
    const neutral = deferred<TestBitmap>();
    const happy = deferred<TestBitmap>();
    const h = harness({
      'asset:C:\\characters\\neutral.png': neutral.promise,
      'asset:C:\\characters\\happy.png': happy.promise,
    });
    const renderer = new StaticImageCharacterRenderer(h.canvas, h.deps);
    renderer.resize(300, 200, 1);
    const loading = renderer.load(manifest);
    neutral.resolve(bitmap('neutral'));
    await Promise.resolve();
    expect(h.drawImage).not.toHaveBeenCalled();
    happy.resolve(bitmap('happy'));
    await loading;
    expect(h.drawImage).toHaveBeenCalledTimes(1);
    expect((h.drawImage.mock.calls[0]?.[0] as TestBitmap).id).toBe('neutral');
  });

  it('switches known expressions atomically and preserves the current frame for unknown names', async () => {
    const neutral = bitmap('neutral');
    const happy = bitmap('happy');
    const h = harness({
      'asset:C:\\characters\\neutral.png': neutral,
      'asset:C:\\characters\\happy.png': happy,
    });
    const renderer = new StaticImageCharacterRenderer(h.canvas, h.deps);
    renderer.resize(300, 200, 1);
    await renderer.load(manifest);
    expect(renderer.setExpression('happy')).toBe(true);
    expect((h.drawImage.mock.lastCall?.[0] as TestBitmap).id).toBe('happy');
    const calls = h.drawImage.mock.calls.length;
    expect(renderer.setExpression('missing')).toBe(false);
    expect(h.drawImage).toHaveBeenCalledTimes(calls);
  });

  it('uses contain geometry with bottom-center alignment and maps alpha threshold hit tests', async () => {
    const h = harness({
      'asset:C:\\characters\\neutral.png': bitmap('neutral'),
      'asset:C:\\characters\\happy.png': bitmap('happy'),
    });
    const renderer = new StaticImageCharacterRenderer(h.canvas, h.deps);
    renderer.resize(300, 200, 2);
    await renderer.load(manifest);
    expect(h.canvas.width).toBe(600);
    expect(h.canvas.height).toBe(400);
    expect(h.setTransform).toHaveBeenLastCalledWith(2, 0, 0, 2, 0, 0);
    expect(h.drawImage).toHaveBeenLastCalledWith(expect.anything(), 50, 0, 200, 200);
    expect(renderer.hitTest(75, 25)).toBe(true);
    expect(renderer.hitTest(175, 25)).toBe(false);
    expect(renderer.hitTest(75, 125)).toBe(false);
    expect(renderer.hitTest(175, 125)).toBe(true);
    expect(renderer.hitTest(49, 100)).toBe(false);
  });

  it('rejects mismatched decoded dimensions and decode failures without drawing', async () => {
    const wrong = harness({
      'asset:C:\\characters\\neutral.png': bitmap('neutral', undefined, 3, 2),
      'asset:C:\\characters\\happy.png': bitmap('happy'),
    });
    const first = new StaticImageCharacterRenderer(wrong.canvas, wrong.deps);
    await expect(first.load(manifest)).rejects.toThrow('dimensions');
    expect(wrong.drawImage).not.toHaveBeenCalled();

    const failed = harness({
      'asset:C:\\characters\\neutral.png': bitmap('neutral'),
      'asset:C:\\characters\\happy.png': Promise.reject(new Error('decode failed')),
    });
    const second = new StaticImageCharacterRenderer(failed.canvas, failed.deps);
    await expect(second.load(manifest)).rejects.toThrow('decode failed');
    expect(failed.drawImage).not.toHaveBeenCalled();
  });

  it('closes every decoded bitmap exactly once and never draws late completions after dispose', async () => {
    const neutral = bitmap('neutral');
    const happy = deferred<TestBitmap>();
    const late = bitmap('happy');
    const h = harness({
      'asset:C:\\characters\\neutral.png': neutral,
      'asset:C:\\characters\\happy.png': happy.promise,
    });
    const renderer = new StaticImageCharacterRenderer(h.canvas, h.deps);
    const loading = renderer.load(manifest);
    await Promise.resolve();
    await Promise.resolve();
    renderer.dispose();
    renderer.dispose();
    happy.resolve(late);
    await loading;
    expect(h.drawImage).not.toHaveBeenCalled();
    expect(neutral.close).toHaveBeenCalledTimes(1);
    expect(late.close).toHaveBeenCalledTimes(1);
  });

  it('returns false for static-only unsupported operations', async () => {
    const h = harness({
      'asset:C:\\characters\\neutral.png': bitmap('neutral'),
      'asset:C:\\characters\\happy.png': bitmap('happy'),
    });
    const renderer = new StaticImageCharacterRenderer(h.canvas, h.deps);
    await renderer.load(manifest);
    expect(renderer.startMotion('idle')).toBe(false);
    expect(renderer.reactToSpeechStart(1)).toBe(false);
    expect(() => renderer.setAudioLevel(0.5)).not.toThrow();
    expect(() => renderer.resetSpeechReaction()).not.toThrow();
  });
});
