import { StrictMode } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type {
  CharacterController,
  CharacterModelSource,
  CharacterRuntimeStatus,
} from '@parallel-world/live2d-runtime';
import { Live2DError } from '@parallel-world/live2d-runtime';
import { createCharacterController, type CharacterModelHandle } from '@parallel-world/live2d-runtime';
import { CharacterCanvas } from './CharacterCanvas';

const source: CharacterModelSource = { modelId: 'mark', manifestUrl: 'models/mark/character.json' };

function controllerDouble(options: { mountError?: unknown; loadError?: unknown } = {}) {
  let listener: ((status: CharacterRuntimeStatus) => void) | undefined;
  const controller: CharacterController = {
    mount: vi.fn(async () => { if (options.mountError) throw options.mountError; }),
    loadModel: vi.fn(async modelSource => {
      listener?.({ kind: 'loading', modelId: modelSource.modelId });
      if (options.loadError) {
        listener?.({ kind: 'failed', modelId: modelSource.modelId, code: 'webgl-unavailable' });
        throw options.loadError;
      }
      listener?.({ kind: 'ready', modelId: modelSource.modelId });
    }),
    playMotion: vi.fn(), setExpression: vi.fn(), resize: vi.fn(), dispose: vi.fn(),
    subscribe: vi.fn(next => { listener = next; next({ kind: 'idle' }); return () => { listener = undefined; }; }),
  };
  return controller;
}

describe('CharacterCanvas', () => {
  it('loadingからreadyへ遷移し、注入した表示文言とmodel sourceを使う', async () => {
    const controller = controllerDouble();
    render(<CharacterCanvas controllerFactory={() => controller} modelSource={source}
      presentation={{ canvasLabel: 'キャラクター描画', loading: '読込中', ready: '表示中', failed: '代替表示' }} />);

    expect(screen.getByRole('status')).toHaveTextContent('読込中');
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('表示中'));
    expect(screen.getByLabelText('キャラクター描画')).toHaveAttribute('data-live2d-state', 'ready');
    expect(controller.loadModel).toHaveBeenCalledWith(source);
    expect(controller.resize).toHaveBeenCalledWith({ cssWidth: 0, cssHeight: 0, devicePixelRatio: 1 });
  });

  it('runtime error codeをcanvasに保持してsilhouette fallbackを表示する', async () => {
    const controller = controllerDouble({ loadError: new Live2DError('webgl-unavailable') });
    render(<CharacterCanvas controllerFactory={() => controller} modelSource={source} />);

    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('代替表示'));
    const canvas = screen.getByLabelText('Live2Dキャラクター');
    expect(canvas).toHaveAttribute('data-live2d-state', 'failed');
    expect(canvas).toHaveAttribute('data-live2d-error-code', 'webgl-unavailable');
    expect(screen.getByTestId('character-silhouette')).toBeVisible();
  });

  it('mount failureもstable error codeへ正規化する', async () => {
    const controller = controllerDouble({ mountError: new Error('bridge missing') });
    render(<CharacterCanvas controllerFactory={() => controller} modelSource={source} />);
    await waitFor(() => expect(screen.getByLabelText('Live2Dキャラクター')).toHaveAttribute('data-live2d-error-code', 'mount-failed'));
  });

  it('StrictModeのmount/unmount/remountごとにcontrollerを一度だけ破棄する', async () => {
    const controllers = [controllerDouble(), controllerDouble()];
    const factory = vi.fn(() => controllers.shift()!);
    const view = render(<StrictMode><CharacterCanvas controllerFactory={factory} modelSource={source} /></StrictMode>);
    await waitFor(() => expect(factory).toHaveBeenCalledTimes(2));
    expect((factory.mock.results[0]!.value as CharacterController).dispose).toHaveBeenCalledTimes(1);
    view.unmount();
    expect((factory.mock.results[1]!.value as CharacterController).dispose).toHaveBeenCalledTimes(1);
  });

  it('unmount後に解決した非同期処理は状態を更新せずcontrollerを再利用しない', async () => {
    let release!: () => void;
    const controller = controllerDouble();
    controller.mount = vi.fn(() => new Promise<void>(resolve => { release = resolve; }));
    const view = render(<CharacterCanvas controllerFactory={() => controller} modelSource={source} />);
    await waitFor(() => expect(controller.mount).toHaveBeenCalledTimes(1));
    view.unmount();
    await act(async () => release());
    expect(controller.loadModel).not.toHaveBeenCalled();
    expect(controller.dispose).toHaveBeenCalledTimes(1);
  });

  it('nonzero CSS sizeとDPR 2を初期同期し、runtime resize通知後もphysical canvasを更新する', async () => {
    const width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(320);
    const height = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(480);
    vi.spyOn(globalThis, 'devicePixelRatio', 'get').mockReturnValue(2);
    let resizeListener: (() => void) | undefined;
    const model: CharacterModelHandle = {
      update: vi.fn(), draw: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), resize: vi.fn(), dispose: vi.fn(),
    };
    const controller = createCharacterController({
      adapter: { mount: vi.fn(), loadModel: vi.fn(async () => model), dispose: vi.fn() },
      environment: {
        requestAnimationFrame: vi.fn(() => 1), cancelAnimationFrame: vi.fn(),
        addResizeListener: vi.fn(listener => { resizeListener = listener; return vi.fn(); }),
      },
      loadManifest: async () => ({ schemaVersion: 1, id: 'mark', model3: 'Mark.model3.json', motions: {}, expressions: {} }),
    });
    const view = render(<CharacterCanvas controllerFactory={() => controller} modelSource={source} />);
    const canvas = screen.getByLabelText('Live2Dキャラクター') as HTMLCanvasElement;
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('表示中'));
    expect([canvas.width, canvas.height]).toEqual([640, 960]);

    width.mockReturnValue(360); height.mockReturnValue(540);
    act(() => resizeListener?.());
    expect([canvas.width, canvas.height]).toEqual([720, 1080]);
    expect(model.resize).toHaveBeenLastCalledWith(720, 1080);
    view.unmount();
  });

  it('unsubscribe例外があってもcontroller disposeを試行する', async () => {
    const controller = controllerDouble();
    controller.subscribe = vi.fn(next => { next({ kind: 'idle' }); return () => { throw new Error('unsubscribe failed'); }; });
    const view = render(<CharacterCanvas controllerFactory={() => controller} modelSource={source} />);
    await waitFor(() => expect(controller.mount).toHaveBeenCalled());
    expect(() => view.unmount()).not.toThrow();
    expect(controller.dispose).toHaveBeenCalledTimes(1);
  });
});
