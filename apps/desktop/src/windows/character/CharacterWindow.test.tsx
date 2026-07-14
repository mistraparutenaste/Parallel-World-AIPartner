import type {
  CharacterManifestDto,
  CharacterSettingsDto,
} from '@parallel-world/contracts';
import { render, screen, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import type { CharacterRenderer } from './character-renderer';
import { CharacterWindow, type CharacterWindowDependencies } from './CharacterWindow';

const STATIC_MANIFEST: CharacterManifestDto = {
  schema_version: 2,
  id: 'epsilon-static',
  display_name: 'Epsilon Static',
  renderer: {
    kind: 'static_image',
    default_expression: 'neutral',
    expressions: [
      { name: 'neutral', image_path: 'neutral.png' },
      { name: 'happy', image_path: 'happy.png' },
    ],
    width: 2,
    height: 2,
  },
};

const SETTINGS: CharacterSettingsDto = {
  schema_version: 1,
  active_character_id: 'epsilon-static',
  expression_idle_timeout_seconds: 20,
};

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => { resolve = yes; });
  return { promise, resolve };
}

function harness(options: {
  manifest?: CharacterManifestDto;
  manifestError?: Error;
  rendererLoad?: Promise<void>;
  settings?: Promise<CharacterSettingsDto>;
} = {}) {
  const handlers = new Map<string, Set<(payload: unknown) => void>>();
  const renderer: CharacterRenderer = {
    kind: options.manifest?.renderer.kind ?? 'static_image',
    load: vi.fn(() => options.rendererLoad ?? Promise.resolve()),
    setExpression: vi.fn(() => true),
    startMotion: vi.fn(() => true),
    setAudioLevel: vi.fn(),
    reactToSpeechStart: vi.fn(() => true),
    resetSpeechReaction: vi.fn(),
    resize: vi.fn(),
    hitTest: vi.fn(() => false),
    dispose: vi.fn(),
  };
  const idle = {
    activity: vi.fn(),
    setConversationState: vi.fn(),
    setAudioActive: vi.fn(),
    setTimeoutSeconds: vi.fn(),
    dispose: vi.fn(),
  };
  let speechOptions: Parameters<CharacterWindowDependencies['createSpeechPlayer']>[0] | undefined;
  let resetDefault: (() => void) | undefined;
  const player = { enqueue: vi.fn(), stop: vi.fn(), dispose: vi.fn() };
  const invokeMock = vi.fn(async (command: string) => {
    if (command === 'get_character_manifest') {
      if (options.manifestError) throw options.manifestError;
      return options.manifest ?? STATIC_MANIFEST;
    }
    if (command === 'get_character_settings') return options.settings ?? SETTINGS;
    return undefined;
  });
  const dependencies: CharacterWindowDependencies = {
    invoke: invokeMock as CharacterWindowDependencies['invoke'],
    convertFileSrc: (path) => `asset:${path}`,
    subscribeEvent: ((name: string, handler: (payload: unknown) => void) => {
      const entries = handlers.get(name) ?? new Set();
      entries.add(handler);
      handlers.set(name, entries);
      return () => entries.delete(handler);
    }) as CharacterWindowDependencies['subscribeEvent'],
    createRenderer: vi.fn(async () => renderer),
    createSpeechPlayer: vi.fn((value) => {
      speechOptions = value;
      return player;
    }),
    createIdleReset: vi.fn((reset) => {
      resetDefault = reset;
      return idle;
    }),
    reportSuccess: vi.fn().mockResolvedValue(undefined),
    createFailureReporter: vi.fn(() => vi.fn()),
    retry: vi.fn().mockResolvedValue(undefined),
  };
  const publish = (name: string, payload: unknown) => {
    for (const handler of handlers.get(name) ?? []) handler(payload);
  };
  return {
    dependencies,
    renderer,
    idle,
    player,
    invokeMock,
    publish,
    handlers,
    get speechOptions() { return speechOptions; },
    get resetDefault() { return resetDefault; },
  };
}

describe('CharacterWindow common renderer lifecycle', () => {
  it('boots a static renderer, queues expression during load, and wires activity without Cubism', async () => {
    const loading = deferred();
    const h = harness({ rendererLoad: loading.promise });
    render(<CharacterWindow dependencies={h.dependencies} />);

    await waitFor(() => expect(h.renderer.load).toHaveBeenCalledOnce());
    h.publish('character-expression', 'happy');
    expect(h.renderer.setExpression).not.toHaveBeenCalled();
    loading.resolve();

    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalledOnce());
    expect(h.dependencies.createRenderer).toHaveBeenCalledWith(
      expect.objectContaining({ renderer: expect.objectContaining({ kind: 'static_image' }) }),
      expect.any(HTMLCanvasElement),
      expect.any(Function),
    );
    expect(h.renderer.setExpression).toHaveBeenCalledWith('happy');
    expect(screen.getByRole('status')).toHaveTextContent(/待機|idle/i);

    h.publish('conversation-state', { schema_version: 1, state: 'thinking', message: null });
    h.publish('character-motion', 'nod');
    const activityBeforeCursor = h.idle.activity.mock.calls.length;
    h.publish('character-cursor', { schema_version: 1, x: 0, y: 0 });
    expect(h.idle.setConversationState).toHaveBeenCalledWith('thinking');
    expect(activityBeforeCursor).toBe(2);
    expect(h.idle.activity).toHaveBeenCalledTimes(activityBeforeCursor);
    expect(h.renderer.hitTest).toHaveBeenCalled();
  });

  it('connects audio level, actual-start hop, stop, settings, and speech activity', async () => {
    const h = harness();
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalledOnce());

    h.publish('speech-audio', { schema_version: 1, turn_id: 7, seq: 0, wav_path: 'speech.wav' });
    expect(h.player.enqueue).toHaveBeenCalledWith({ turnId: 7, seq: 0, url: 'asset:speech.wav' });
    h.speechOptions?.onLevel?.(0.4);
    h.speechOptions?.onActiveChange?.(true);
    h.speechOptions?.onTurnPlaybackStart?.(7);
    expect(h.renderer.setAudioLevel).toHaveBeenCalledWith(0.4);
    expect(h.idle.setAudioActive).toHaveBeenCalledWith(true);
    expect(h.renderer.reactToSpeechStart).toHaveBeenCalledWith(7);

    h.publish('speech-stop', { schema_version: 1, turn_id: 7 });
    expect(h.player.stop).toHaveBeenCalledOnce();
    expect(h.renderer.resetSpeechReaction).toHaveBeenCalledOnce();
    h.publish('character-settings-changed', { schema_version: 1, settings: { ...SETTINGS, expression_idle_timeout_seconds: null } });
    expect(h.idle.setTimeoutSeconds).toHaveBeenLastCalledWith(null);
  });

  it('does not overwrite a newer settings event with a stale initial response', async () => {
    let resolveSettings!: (settings: CharacterSettingsDto) => void;
    const settings = new Promise<CharacterSettingsDto>((resolve) => { resolveSettings = resolve; });
    const h = harness({ settings });
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.invokeMock).toHaveBeenCalledWith('get_character_settings'));
    h.publish('character-settings-changed', {
      schema_version: 1,
      settings: { ...SETTINGS, expression_idle_timeout_seconds: null },
    });
    resolveSettings(SETTINGS);
    await Promise.resolve();

    expect(h.idle.setTimeoutSeconds).toHaveBeenCalledWith(null);
    expect(h.idle.setTimeoutSeconds).not.toHaveBeenCalledWith(20);
  });

  it('idle reset does not count its own default-expression write as new activity', async () => {
    const h = harness();
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalledOnce());
    vi.mocked(h.renderer.setExpression).mockClear();
    h.idle.activity.mockClear();

    h.resetDefault?.();

    expect(h.renderer.setExpression).toHaveBeenCalledWith('neutral');
    expect(h.idle.activity).not.toHaveBeenCalled();
  });

  it('re-arms the idle deadline when a slow renderer becomes ready', async () => {
    const loading = deferred();
    const h = harness({ rendererLoad: loading.promise });
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.renderer.load).toHaveBeenCalledOnce());
    expect(h.idle.activity).not.toHaveBeenCalled();

    loading.resolve();
    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalledOnce());
    expect(h.idle.activity).toHaveBeenCalledOnce();
  });

  it('disposes a renderer whose load fails and keeps a successful surface if health reporting fails', async () => {
    const loadFailure = harness({ rendererLoad: Promise.reject(new Error('decode failed')) });
    render(<CharacterWindow dependencies={loadFailure.dependencies} />);
    await screen.findByText(/decode failed/);
    expect(loadFailure.renderer.dispose).toHaveBeenCalledOnce();

    const healthFailure = harness();
    vi.mocked(healthFailure.dependencies.reportSuccess).mockRejectedValue(new Error('health unavailable'));
    render(<CharacterWindow dependencies={healthFailure.dependencies} />);
    await waitFor(() => expect(healthFailure.dependencies.reportSuccess).toHaveBeenCalledOnce());
    expect(healthFailure.renderer.dispose).not.toHaveBeenCalled();
    expect(screen.getAllByRole('status').at(-1)).toHaveTextContent(/待機|idle/i);
  });

  it('treats invalid profile selection as permanent, hides only the surface, and does not auto retry', async () => {
    const h = harness({ manifestError: new Error('selection_required') });
    render(<CharacterWindow dependencies={h.dependencies} />);

    expect(await screen.findByText(/チャットは通常どおり/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '設定修正後に再読み込み' })).toBeInTheDocument();
    expect(document.querySelector('[data-character-surface]')).not.toBeVisible();
    expect(h.dependencies.createRenderer).not.toHaveBeenCalled();
    expect(h.dependencies.retry).not.toHaveBeenCalled();

    h.publish('speech-audio', { schema_version: 1, turn_id: 9, seq: 0, wav_path: 'fallback.wav' });
    h.publish('speech-stop', { schema_version: 1, turn_id: 9 });
    expect(h.player.enqueue).toHaveBeenCalledWith({
      turnId: 9, seq: 0, url: 'asset:fallback.wav',
    });
    expect(h.player.stop).toHaveBeenCalledOnce();
  });

  it('cancels an active speech reaction when conversation enters interrupting', async () => {
    const h = harness();
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalledOnce());
    h.publish('conversation-state', {
      schema_version: 1, state: 'interrupting', message: null,
    });
    expect(h.renderer.resetSpeechReaction).toHaveBeenCalledOnce();
  });

  it('reboots only after the supervisor emits a transient retry start', async () => {
    const h = harness();
    render(<CharacterWindow dependencies={h.dependencies} />);
    await waitFor(() => expect(h.dependencies.createRenderer).toHaveBeenCalledOnce());

    h.publish('runtime-health', {
      schema_version: 1,
      feature: 'character_renderer',
      status: 'starting',
      failure_class: null,
      last_error: null,
      attempts: 1,
      ownership: 'not_applicable',
      circuit_open: false,
      changed_at_ms: 1,
    });

    await waitFor(() => expect(h.dependencies.createRenderer).toHaveBeenCalledTimes(2));
    expect(h.renderer.dispose).toHaveBeenCalled();
  });

  it('StrictMode setup-cleanup-setup retains one subscription owner and disposes every owner', async () => {
    const h = harness();
    const view = render(
      <StrictMode><CharacterWindow dependencies={h.dependencies} /></StrictMode>,
    );
    await waitFor(() => expect(h.dependencies.reportSuccess).toHaveBeenCalled());
    for (const entries of h.handlers.values()) expect(entries.size).toBe(1);

    view.unmount();
    for (const entries of h.handlers.values()) expect(entries.size).toBe(0);
    expect(h.renderer.dispose).toHaveBeenCalled();
    expect(h.player.dispose).toHaveBeenCalled();
    expect(h.idle.dispose).toHaveBeenCalled();
  });
});
