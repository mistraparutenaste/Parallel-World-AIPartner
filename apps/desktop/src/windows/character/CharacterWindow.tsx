import type {
  CharacterCursorEventDto,
  CharacterManifestDto,
  ConversationStateDto,
  SpeechAudioEventDto,
  SpeechStopEventDto,
} from '@parallel-world/contracts';
import {
  Live2DController,
  type Live2DControllerState,
  SpeechAudioPlayer,
  WebAudioSink,
} from '@parallel-world/live2d-runtime';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { StatusBadge } from '../../shared/components/StatusBadge';
import { createModelSource } from './model-source';
import { reportLive2DFailure, reportLive2DSuccess, rearmLive2D } from './live2d-health';

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
  const [retryGeneration, setRetryGeneration] = useState(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return undefined;
    }
    let disposed = false;
    let controller: Live2DController | null = null;
    let player: SpeechAudioPlayer | null = null;
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
        void reportLive2DFailure('core_missing');
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
        (nextState) => {
          setState(nextState);
          if (nextState === 'model-loaded') void reportLive2DSuccess();
          if (nextState === 'unavailable') void reportLive2DFailure('renderer_initialization_failed');
        },
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
        void reportLive2DFailure('model_load_failed');
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
      // Speech playback: synthesized sentences arrive as WAV paths and
      // play in order; the measured level drives the mouth parameters.
      // While audio is active the microphone capture is muted so the
      // assistant does not hear itself.
      const audioPlayer = new SpeechAudioPlayer(new WebAudioSink(), {
        onLevel: (level) => {
          instance.setLipSyncValue(level);
        },
        onActiveChange: (active) => {
          invoke('set_speech_playback', { active }).catch((error: unknown) => {
            console.error('failed to report speech playback', error);
          });
        },
      });
      player = audioPlayer;
      const stopAudio = subscribeEvent<SpeechAudioEventDto>(
        'speech-audio',
        (payload) => {
          audioPlayer.enqueue({
            turnId: payload.turn_id,
            seq: payload.seq,
            url: convertFileSrc(payload.wav_path),
          });
        },
      );
      const stopSpeech = subscribeEvent<SpeechStopEventDto>(
        'speech-stop',
        () => {
          audioPlayer.stop();
        },
      );
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
        stopAudio();
        stopSpeech();
        stopCursor();
        audioPlayer.dispose();
      } else {
        unlisteners.push(stopExpression, stopMotion, stopAudio, stopSpeech, stopCursor);
      }
    };
    void boot().catch((error: unknown) => {
      console.error('failed to initialize Live2D', error);
      setLoadError(String(error));
      setState('unavailable');
      void reportLive2DFailure('renderer_initialization_failed');
    });

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
      player?.dispose();
      controller?.dispose();
    };
  }, [retryGeneration]);

  return (
    <main aria-label="キャラクター">
      <canvas
        ref={canvasRef}
        className="character-canvas"
        data-tauri-drag-region
        data-live2d-surface
        hidden={state === 'unavailable'}
      />
      <div className="character-status">
        <StatusBadge state={toBadgeState(state)} />
        {loadError !== null && (
          <p className="character-error">{loadError}</p>
        )}
        {state === 'unavailable' && (
          <section aria-label="キャラクター表示フォールバック">
            <p>キャラクター表示を利用できません。チャットは通常どおり利用できます。</p>
            <button type="button" onClick={() => {
              void rearmLive2D();
              setLoadError(null);
              setState('idle');
              setRetryGeneration((generation) => generation + 1);
            }}>再試行</button>
          </section>
        )}
      </div>
    </main>
  );
}
