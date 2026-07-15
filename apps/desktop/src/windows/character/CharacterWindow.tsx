import type {
  CharacterCursorEventDto,
  CharacterManifestDto,
  CharacterSettingsChangedEventDto,
  CharacterSettingsDto,
  ConversationStateDto,
  ConversationStateEventDto,
  RuntimeHealthEventDto,
  SpeechAudioEventDto,
  SpeechStopEventDto,
} from '@parallel-world/contracts';
import {
  Live2DController,
  SpeechAudioPlayer,
  WebAudioSink,
  type SpeechAudioPlayerOptions,
} from '@parallel-world/live2d-runtime';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { StatusBadge } from '../../shared/components/StatusBadge';
import { subscribeEvent } from '../../shared/ipc/event-bus';
import { CharacterIdleResetController } from './character-idle-reset';
import type { CharacterRenderer } from './character-renderer';
import { createCharacterRenderer } from './character-renderer-factory';
import {
  classifyCharacterRendererFailure,
  classifyCharacterRendererLoadFailure,
  createCharacterRendererFailureReporter,
  isPermanentCharacterRendererFailure,
  reportCharacterRendererSuccess,
  retryCharacterRenderer,
  type CharacterRendererFailureCode,
} from './character-renderer-health';
import { SpeechHopController } from './speech-hop';

const SHADER_PATH = '/live2d/shaders/';

type SurfaceState = 'starting' | 'ready' | 'unavailable';

interface SpeechPlayerLike {
  enqueue(item: { turnId: number; seq: number; url: string }): void;
  stop(): void;
  dispose(): void;
}

interface IdleResetLike {
  activity(): void;
  setConversationState(state: ConversationStateDto): void;
  setAudioActive(active: boolean): void;
  setTimeoutSeconds(value: number | null): void;
  dispose(): void;
}

export interface CharacterWindowDependencies {
  invoke: typeof invoke;
  convertFileSrc: typeof convertFileSrc;
  subscribeEvent: typeof subscribeEvent;
  createRenderer(
    manifest: CharacterManifestDto,
    canvas: HTMLCanvasElement,
    onControllerState: (state: string) => void,
  ): Promise<CharacterRenderer>;
  createSpeechPlayer(options: SpeechAudioPlayerOptions): SpeechPlayerLike;
  createIdleReset(reset: () => void): IdleResetLike;
  reportSuccess(): Promise<void>;
  createFailureReporter(): (code: CharacterRendererFailureCode) => void;
  retry(): Promise<void>;
}

const DEFAULT_DEPENDENCIES: CharacterWindowDependencies = {
  invoke,
  convertFileSrc,
  subscribeEvent,
  async createRenderer(manifest, canvas, onControllerState) {
    if (manifest.renderer.kind === 'static_image') {
      const hop = new SpeechHopController(canvas);
      return createCharacterRenderer(manifest.renderer, {
        canvas,
        convertFileSrc,
        staticImage: { speechReaction: { react: (turnId) => hop.react(turnId), reset: () => hop.cancel() } },
      });
    }

    if (!('Live2DCubismCore' in globalThis)) throw new Error('core_missing');
    const { CubismFrameworkRuntime } = await import('@parallel-world/live2d-runtime/cubism');
    return createCharacterRenderer(manifest.renderer, {
      canvas,
      convertFileSrc,
      createLive2DController: () => new Live2DController(
        new CubismFrameworkRuntime({ shaderPath: SHADER_PATH }),
        onControllerState,
      ),
    });
  },
  createSpeechPlayer: (options) => new SpeechAudioPlayer(new WebAudioSink(), options),
  createIdleReset: (reset) => new CharacterIdleResetController(reset),
  reportSuccess: reportCharacterRendererSuccess,
  createFailureReporter: createCharacterRendererFailureReporter,
  retry: retryCharacterRenderer,
};

function badgeState(state: SurfaceState): ConversationStateDto {
  if (state === 'ready') return 'idle';
  if (state === 'unavailable') return 'renderer_unavailable';
  return 'starting';
}

function expressionNames(manifest: CharacterManifestDto): ReadonlySet<string> {
  return new Set(manifest.renderer.kind === 'static_image'
    ? manifest.renderer.expressions.map(({ name }) => name)
    : manifest.renderer.expressions);
}

function defaultExpression(manifest: CharacterManifestDto): string | null {
  return manifest.renderer.default_expression;
}

/** Owns the renderer, speech, timers and all character-window subscriptions. */
export function CharacterWindow({
  dependencies = DEFAULT_DEPENDENCIES,
}: { dependencies?: CharacterWindowDependencies } = {}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<SurfaceState>('starting');
  const [loadError, setLoadError] = useState<string | null>(null);
  const [permanentFailure, setPermanentFailure] = useState(false);
  const [retryGeneration, setRetryGeneration] = useState(0);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return undefined;

    let disposed = false;
    let loaded = false;
    let renderer: CharacterRenderer | null = null;
    let player: SpeechPlayerLike | null = null;
    let idle: IdleResetLike | null = null;
    let pendingExpression: string | null = null;
    let manifest: CharacterManifestDto | null = null;
    let bootedCharacterId: string | null = null;
    let requestedCharacterId: string | null | undefined;
    let interactive = true;
    let settingsRevision = 0;
    const unlisteners: Array<() => void> = [];
    const reportFailureOnce = dependencies.createFailureReporter();

    const recordFailure = (error: unknown, fallback?: CharacterRendererFailureCode) => {
      if (disposed) return;
      const code = fallback ?? classifyCharacterRendererFailure(error);
      setLoadError(String(error));
      setPermanentFailure(isPermanentCharacterRendererFailure(code));
      setState('unavailable');
      reportFailureOnce(code);
    };

    const resize = () => renderer?.resize(
      canvas.clientWidth,
      canvas.clientHeight,
      window.devicePixelRatio,
    );

    // Speech remains available even when the profile is missing or invalid.
    // Renderer/idle callbacks are optional until a character finishes booting.
    player = dependencies.createSpeechPlayer({
      onLevel: (level) => renderer?.setAudioLevel(level),
      onActiveChange: (active) => {
        idle?.setAudioActive(active);
        void dependencies.invoke('set_speech_playback', { active }).catch(
          (error: unknown) => console.error('failed to report speech playback', error),
        );
      },
      onTurnPlaybackStart: (turnId) => {
        renderer?.reactToSpeechStart(turnId);
        idle?.activity();
      },
    });
    const audioPlayer = player;
    unlisteners.push(
      dependencies.subscribeEvent<SpeechAudioEventDto>('speech-audio', (payload) => {
        audioPlayer.enqueue({
          turnId: payload.turn_id,
          seq: payload.seq,
          url: dependencies.convertFileSrc(payload.wav_path),
        });
      }),
      dependencies.subscribeEvent<SpeechStopEventDto>('speech-stop', () => {
        audioPlayer.stop();
        renderer?.resetSpeechReaction();
        idle?.activity();
      }),
    );

    unlisteners.push(dependencies.subscribeEvent<RuntimeHealthEventDto>(
      'runtime-health',
      (event) => {
        if (
          event.feature === 'character_renderer'
          && event.status === 'starting'
          && !disposed
        ) {
          setLoadError(null);
          setPermanentFailure(false);
          setState('starting');
          setRetryGeneration((generation) => generation + 1);
        }
      },
    ));

    unlisteners.push(dependencies.subscribeEvent<CharacterSettingsChangedEventDto>(
      'character-settings-changed',
      ({ settings }) => {
        settingsRevision += 1;
        idle?.setTimeoutSeconds(settings.expression_idle_timeout_seconds);
        const lifecycleCharacterId = requestedCharacterId === undefined
          ? bootedCharacterId
          : requestedCharacterId;
        if (settings.active_character_id === lifecycleCharacterId) return;
        requestedCharacterId = settings.active_character_id;
        setLoadError(null);
        setPermanentFailure(false);
        setState('starting');
        setRetryGeneration((generation) => generation + 1);
      },
    ));

    const boot = async () => {
      try {
        manifest = await dependencies.invoke<CharacterManifestDto>('get_character_manifest');
      } catch (error) {
        recordFailure(error);
        return;
      }
      if (disposed) return;
      bootedCharacterId = manifest.id;
      requestedCharacterId = manifest.id;

      const names = expressionNames(manifest);
      const resetExpression = () => {
        const name = manifest === null ? null : defaultExpression(manifest);
        if (name !== null) renderer?.setExpression(name);
      };
      idle = dependencies.createIdleReset(resetExpression);

      unlisteners.push(
        dependencies.subscribeEvent<string>('character-expression', (name) => {
          if (!names.has(name)) return;
          if (!loaded) pendingExpression = name;
          else if (renderer?.setExpression(name)) idle?.activity();
        }),
        dependencies.subscribeEvent<string>('character-motion', (group) => {
          if (loaded && renderer?.startMotion(group)) idle?.activity();
        }),
        dependencies.subscribeEvent<ConversationStateEventDto>(
          'conversation-state',
          ({ state: conversationState }) => {
            idle?.setConversationState(conversationState);
            if (conversationState === 'interrupting') renderer?.resetSpeechReaction();
          },
        ),
        dependencies.subscribeEvent<CharacterCursorEventDto>('character-cursor', (payload) => {
          const overCharacter = loaded ? (renderer?.hitTest(payload.x, payload.y) ?? false) : true;
          if (overCharacter === interactive) return;
          interactive = overCharacter;
          void dependencies.invoke('set_click_through', { enabled: !overCharacter }).catch(
            (error: unknown) => console.error('failed to toggle click-through', error),
          );
        }),
      );

      try {
        const nextRenderer = await dependencies.createRenderer(
          manifest,
          canvas,
          (controllerState) => {
            if (controllerState === 'unavailable') {
              recordFailure(new Error('renderer initialization failed'));
            }
          },
        );
        if (disposed) {
          nextRenderer.dispose();
          return;
        }
        renderer = nextRenderer;
        resize();
        await nextRenderer.load(manifest.renderer);
        if (disposed) return;
        loaded = true;
        if (pendingExpression !== null) nextRenderer.setExpression(pendingExpression);
        pendingExpression = null;
        resize();
        idle?.activity();
        setState('ready');
        setLoadError(null);
        setPermanentFailure(false);
      } catch (error) {
        renderer?.dispose();
        renderer = null;
        recordFailure(
          error,
          classifyCharacterRendererLoadFailure(manifest.renderer.kind, error),
        );
        return;
      }
      void dependencies.reportSuccess().catch((error: unknown) => {
        console.warn('failed to report character renderer health', error);
      });

      try {
        const requestedAtRevision = settingsRevision;
        const settings = await dependencies.invoke<CharacterSettingsDto>('get_character_settings');
        if (!disposed && settingsRevision === requestedAtRevision) {
          idle?.setTimeoutSeconds(settings.expression_idle_timeout_seconds);
        }
      } catch (error) {
        console.warn('failed to load character settings; using the default timeout', error);
      }
    };

    window.addEventListener('resize', resize);
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(resize);
    observer?.observe(canvas);
    void boot().catch((error: unknown) => recordFailure(error));

    return () => {
      disposed = true;
      observer?.disconnect();
      window.removeEventListener('resize', resize);
      for (let index = unlisteners.length - 1; index >= 0; index -= 1) {
        unlisteners[index]?.();
      }
      idle?.dispose();
      player?.dispose();
      renderer?.dispose();
    };
  }, [dependencies, retryGeneration]);

  return (
    <main aria-label="キャラクター">
      <canvas
        ref={canvasRef}
        className="character-canvas"
        data-tauri-drag-region
        data-character-surface
        hidden={state === 'unavailable'}
      />
      <div className="character-status">
        <StatusBadge state={badgeState(state)} />
        {loadError !== null ? <p className="character-error">{loadError}</p> : null}
        {state === 'unavailable' ? (
          <section aria-label="キャラクター表示フォールバック">
            <p>キャラクター表示を利用できません。チャットは通常どおり利用できます。</p>
            <button
              type="button"
              onClick={() => {
                void dependencies.retry().catch((error: unknown) => setLoadError(String(error)));
              }}
            >
              {permanentFailure ? '設定修正後に再読み込み' : '再試行'}
            </button>
          </section>
        ) : null}
      </div>
    </main>
  );
}
