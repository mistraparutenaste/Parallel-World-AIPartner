import type {
  CharacterCursorEventDto,
  CharacterManifestDto,
  ConversationStateDto,
} from '@parallel-world/contracts';
import {
  Live2DController,
  type Live2DControllerState,
} from '@parallel-world/live2d-runtime';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { StatusBadge } from '../../shared/components/StatusBadge';
import { createModelSource } from './model-source';

const SHADER_PATH = '/live2d/shaders/';

function toBadgeState(state: Live2DControllerState): ConversationStateDto {
  switch (state) {
    case 'model-loaded':
      return 'idle';
    case 'unavailable':
      return 'renderer_unavailable';
    default:
      return 'starting';
  }
}

/**
 * Transparent always-on-top character surface.
 *
 * Owns one Live2DController per mount. The Cubism adapter is loaded
 * dynamically only after the Core script global is detected, because
 * the vendored framework cannot even be imported without it.
 * Expression / motion commands arrive as Tauri events emitted by the
 * Rust side after validation.
 */
export function CharacterWindow() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<Live2DControllerState>('idle');
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return undefined;
    }
    let disposed = false;
    let controller: Live2DController | null = null;
    const unlisteners: Array<() => void> = [];

    const resize = () => {
      controller?.resize(
        canvas.clientWidth,
        canvas.clientHeight,
        window.devicePixelRatio,
      );
    };

    const boot = async () => {
      if (!('Live2DCubismCore' in globalThis)) {
        setLoadError('Cubism Core script is not loaded');
        setState('unavailable');
        return;
      }
      const { CubismFrameworkRuntime } = await import(
        '@parallel-world/live2d-runtime/cubism'
      );
      if (disposed) {
        return;
      }
      const instance = new Live2DController(
        new CubismFrameworkRuntime({ shaderPath: SHADER_PATH }),
        setState,
      );
      controller = instance;
      await instance.attach(canvas);
      if (disposed || instance.state !== 'ready') {
        return;
      }
      resize();
      try {
        const manifest = await invoke<CharacterManifestDto>(
          'get_character_manifest',
        );
        if (disposed) {
          return;
        }
        await instance.loadModel(
          createModelSource(manifest.model_path, convertFileSrc),
        );
      } catch (error) {
        console.error('failed to load the character model', error);
        setLoadError(String(error));
        setState('unavailable');
        return;
      }
      const stopExpression = subscribeEvent<string>(
        'character-expression',
        (payload) => {
          instance.setExpression(payload);
        },
      );
      const stopMotion = subscribeEvent<string>('character-motion', (payload) => {
        instance.startMotion(payload);
      });
      // Click-through: the Rust cursor watcher streams positions even
      // while mouse events are ignored; clicks pass through unless the
      // cursor is over an opaque model pixel.
      let interactive = true;
      const stopCursor = subscribeEvent<CharacterCursorEventDto>(
        'character-cursor',
        (payload) => {
          const overModel =
            instance.state === 'model-loaded'
              ? instance.hitTest(payload.x, payload.y)
              : true;
          if (overModel !== interactive) {
            interactive = overModel;
            invoke('set_click_through', { enabled: !overModel }).catch(
              (error: unknown) => {
                console.error('failed to toggle click-through', error);
              },
            );
          }
        },
      );
      if (disposed) {
        stopExpression();
        stopMotion();
        stopCursor();
      } else {
        unlisteners.push(stopExpression, stopMotion, stopCursor);
      }
    };
    void boot();

    window.addEventListener('resize', resize);
    const observer =
      typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(resize);
    observer?.observe(canvas);

    return () => {
      disposed = true;
      observer?.disconnect();
      window.removeEventListener('resize', resize);
      for (const unlisten of unlisteners) {
        unlisten();
      }
      controller?.dispose();
    };
  }, []);

  return (
    <main aria-label="キャラクター">
      <canvas
        ref={canvasRef}
        className="character-canvas"
        data-tauri-drag-region
        data-live2d-surface
      />
      <div className="character-status">
        <StatusBadge state={toBadgeState(state)} />
        {loadError !== null && (
          <p className="character-error">{loadError}</p>
        )}
      </div>
    </main>
  );
}
