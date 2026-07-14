import type { CharacterRendererDto } from '@parallel-world/contracts';
import { describe, expect, it, vi } from 'vitest';
import { createCharacterRenderer } from './character-renderer-factory';
import { Live2DCharacterRenderer } from './live2d-character-renderer';
import { StaticImageCharacterRenderer } from './static-image-character-renderer';

const canvas = { getContext: vi.fn(() => ({ clearRect: vi.fn(), drawImage: vi.fn(), setTransform: vi.fn() })) } as unknown as HTMLCanvasElement;

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => { resolve = yes; });
  return { promise, resolve };
}

describe('createCharacterRenderer', () => {
  it('selects static without evaluating the Live2D dependency', () => {
    const deps = {
      canvas,
      convertFileSrc: (path: string) => path,
      get createLive2DController(): never {
        throw new Error('Cubism path evaluated');
      },
    };
    const renderer = createCharacterRenderer({
      kind: 'static_image',
      default_expression: 'neutral',
      expressions: [{ name: 'neutral', image_path: 'neutral.png' }],
      width: 1,
      height: 1,
    }, deps);
    expect(renderer).toBeInstanceOf(StaticImageCharacterRenderer);
    expect(renderer.kind).toBe('static_image');
  });

  it('selects the Live2D adapter and delegates controller behavior', async () => {
    const controller = {
      state: 'idle' as const,
      attach: vi.fn().mockResolvedValue(undefined),
      loadModel: vi.fn().mockResolvedValue(undefined),
      setExpression: vi.fn(() => true),
      startMotion: vi.fn(() => true),
      setLipSyncValue: vi.fn(() => true),
      resize: vi.fn(),
      hitTest: vi.fn(() => true),
      dispose: vi.fn(),
    };
    const dto: Extract<CharacterRendererDto, { kind: 'live2d' }> = {
      kind: 'live2d',
      model_path: 'C:\\model\\avatar.model3.json',
      default_expression: null,
      expressions: [],
      motion_groups: [],
    };
    const renderer = createCharacterRenderer(dto, {
      canvas,
      convertFileSrc: (path) => `asset:${path}`,
      createLive2DController: vi.fn(() => controller),
    });
    expect(renderer).toBeInstanceOf(Live2DCharacterRenderer);
    await renderer.load(dto);
    expect(controller.attach).toHaveBeenCalledWith(canvas);
    expect(controller.loadModel).toHaveBeenCalledWith(expect.objectContaining({
      modelUrl: 'asset:C:\\model\\avatar.model3.json',
    }));
    expect(renderer.setExpression('happy')).toBe(true);
    expect(renderer.startMotion('idle')).toBe(true);
    renderer.setAudioLevel(0.4);
    renderer.resize(100, 200, 1.5);
    expect(renderer.hitTest(10, 20)).toBe(true);
    expect(renderer.reactToSpeechStart(7)).toBe(false);
    renderer.resetSpeechReaction();
    renderer.dispose();
    renderer.dispose();
    expect(controller.setExpression).toHaveBeenCalledWith('happy');
    expect(controller.startMotion).toHaveBeenCalledWith('idle');
    expect(controller.setLipSyncValue).toHaveBeenCalledWith(0.4);
    expect(controller.resize).toHaveBeenCalledWith(100, 200, 1.5);
    expect(controller.hitTest).toHaveBeenCalledWith(10, 20);
    expect(controller.dispose).toHaveBeenCalledTimes(1);
  });

  it('re-disposes and aborts when attach completes after renderer disposal', async () => {
    const attached = deferred();
    let state: 'idle' | 'ready' | 'disposed' = 'idle';
    const controller = {
      get state() { return state; },
      attach: vi.fn(async () => { await attached.promise; state = 'ready'; }),
      loadModel: vi.fn().mockResolvedValue(undefined),
      setExpression: vi.fn(() => false),
      startMotion: vi.fn(() => false),
      setLipSyncValue: vi.fn(() => false),
      resize: vi.fn(),
      hitTest: vi.fn(() => false),
      dispose: vi.fn(() => { state = 'disposed'; }),
    };
    const dto: Extract<CharacterRendererDto, { kind: 'live2d' }> = {
      kind: 'live2d', model_path: 'avatar.model3.json', default_expression: null,
      expressions: [], motion_groups: [],
    };
    const renderer = createCharacterRenderer(dto, {
      canvas, convertFileSrc: (path) => path, createLive2DController: () => controller,
    });
    const loading = renderer.load(dto);
    await Promise.resolve();
    renderer.dispose();
    attached.resolve();
    await expect(loading).rejects.toThrow('disposed');
    expect(controller.loadModel).not.toHaveBeenCalled();
    expect(controller.dispose).toHaveBeenCalledTimes(2);
    expect(state).toBe('disposed');
  });

  it('re-disposes and aborts when model loading completes after renderer disposal', async () => {
    const modelLoaded = deferred();
    let state: 'idle' | 'ready' | 'model-loaded' | 'disposed' = 'idle';
    const controller = {
      get state() { return state; },
      attach: vi.fn(async () => { state = 'ready'; }),
      loadModel: vi.fn(async () => { await modelLoaded.promise; state = 'model-loaded'; }),
      setExpression: vi.fn(() => false),
      startMotion: vi.fn(() => false),
      setLipSyncValue: vi.fn(() => false),
      resize: vi.fn(),
      hitTest: vi.fn(() => false),
      dispose: vi.fn(() => { state = 'disposed'; }),
    };
    const dto: Extract<CharacterRendererDto, { kind: 'live2d' }> = {
      kind: 'live2d', model_path: 'avatar.model3.json', default_expression: null,
      expressions: [], motion_groups: [],
    };
    const renderer = createCharacterRenderer(dto, {
      canvas, convertFileSrc: (path) => path, createLive2DController: () => controller,
    });
    const loading = renderer.load(dto);
    await vi.waitFor(() => expect(controller.loadModel).toHaveBeenCalledOnce());
    renderer.dispose();
    modelLoaded.resolve();
    await expect(loading).rejects.toThrow('disposed');
    expect(controller.dispose).toHaveBeenCalledTimes(2);
    expect(state).toBe('disposed');
  });

  it('fails closed for an unknown renderer kind', () => {
    expect(() => createCharacterRenderer(
      { kind: 'future_renderer' } as unknown as CharacterRendererDto,
      { canvas, convertFileSrc: (path) => path },
    )).toThrow('unsupported character renderer kind');
  });
});
