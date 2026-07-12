import { describe, expect, it, vi } from 'vitest';
import type {
  CubismRuntime,
  ModelHandle,
  ModelSource,
} from '../runtime/cubism-runtime';
import { Live2DController } from './live2d-controller';

function createModelHandle(): ModelHandle {
  return {
    expressions: ['Normal', 'Smile'],
    motionGroups: new Map([['Idle', 1]]),
    setExpression: vi.fn().mockReturnValue(true),
    startMotion: vi.fn().mockReturnValue(true),
    hitTest: vi.fn().mockReturnValue(false),
    release: vi.fn(),
  };
}

function createRuntime(overrides: Partial<CubismRuntime> = {}): CubismRuntime {
  return {
    start: vi.fn().mockResolvedValue(undefined),
    loadModel: vi.fn().mockResolvedValue(createModelHandle()),
    resize: vi.fn(),
    stop: vi.fn(),
    ...overrides,
  };
}

function createSource(): ModelSource {
  return {
    modelUrl: '/model.model3.json',
    resolveResource: (relative) => `/${relative}`,
  };
}

function createCanvas(): HTMLCanvasElement {
  return document.createElement('canvas');
}

describe('Live2DController', () => {
  it('reaches ready state after attaching to a canvas', async () => {
    const states: string[] = [];
    const controller = new Live2DController(createRuntime(), (state) => {
      states.push(state);
    });

    await controller.attach(createCanvas());

    expect(controller.state).toBe('ready');
    expect(states).toEqual(['starting', 'ready']);
  });

  it('becomes unavailable when the runtime cannot start', async () => {
    const runtime = createRuntime({
      start: vi.fn().mockRejectedValue(new Error('core missing')),
    });
    const controller = new Live2DController(runtime);

    await controller.attach(createCanvas());

    expect(controller.state).toBe('unavailable');
  });

  it('rejects loading a model before attach', async () => {
    const controller = new Live2DController(createRuntime());

    await expect(controller.loadModel(createSource())).rejects.toThrow(
      'not ready',
    );
  });

  it('exposes expressions and motion groups after loading a model', async () => {
    const controller = new Live2DController(createRuntime());
    await controller.attach(createCanvas());

    await controller.loadModel(createSource());

    expect(controller.state).toBe('model-loaded');
    expect(controller.expressions).toEqual(['Normal', 'Smile']);
    expect(controller.motionGroups.get('Idle')).toBe(1);
  });

  it('becomes unavailable when the model cannot be loaded', async () => {
    const runtime = createRuntime({
      loadModel: vi.fn().mockRejectedValue(new Error('404')),
    });
    const controller = new Live2DController(runtime);
    await controller.attach(createCanvas());

    await expect(controller.loadModel(createSource())).rejects.toThrow(
      '404',
    );
    expect(controller.state).toBe('unavailable');
  });

  it('ignores expression and motion requests without a model', async () => {
    const controller = new Live2DController(createRuntime());
    await controller.attach(createCanvas());

    expect(controller.setExpression('Smile')).toBe(false);
    expect(controller.startMotion('Idle')).toBe(false);
  });

  it('delegates expression and motion requests to the model', async () => {
    const handle = createModelHandle();
    const runtime = createRuntime({
      loadModel: vi.fn().mockResolvedValue(handle),
    });
    const controller = new Live2DController(runtime);
    await controller.attach(createCanvas());
    await controller.loadModel(createSource());

    expect(controller.setExpression('Smile')).toBe(true);
    expect(handle.setExpression).toHaveBeenCalledWith('Smile');
    expect(controller.startMotion('Idle')).toBe(true);
    expect(handle.startMotion).toHaveBeenCalledWith('Idle', undefined);
  });

  it('resizes the canvas backing store by device pixel ratio', async () => {
    const runtime = createRuntime();
    const controller = new Live2DController(runtime);
    const canvas = createCanvas();
    await controller.attach(canvas);

    controller.resize(400, 300, 2);

    expect(canvas.width).toBe(800);
    expect(canvas.height).toBe(600);
    expect(runtime.resize).toHaveBeenCalledWith(800, 600);
  });

  it('releases the model and stops the runtime on dispose', async () => {
    const handle = createModelHandle();
    const runtime = createRuntime({
      loadModel: vi.fn().mockResolvedValue(handle),
    });
    const controller = new Live2DController(runtime);
    await controller.attach(createCanvas());
    await controller.loadModel(createSource());

    controller.dispose();

    expect(handle.release).toHaveBeenCalled();
    expect(runtime.stop).toHaveBeenCalled();
    expect(controller.state).toBe('disposed');
  });

  it('dispose is idempotent and attach afterwards is rejected', async () => {
    const controller = new Live2DController(createRuntime());
    await controller.attach(createCanvas());

    controller.dispose();
    controller.dispose();

    await expect(controller.attach(createCanvas())).rejects.toThrow('disposed');
  });
});
