import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PlaybackRequest } from './speech-audio-player';
import { WebAudioSink } from './web-audio-sink';

function audioHarness(start: () => void) {
  const source = {
    buffer: null as AudioBuffer | null,
    connect: vi.fn(),
    start: vi.fn(start),
    stop: vi.fn(),
    onended: null as (() => void) | null,
  };
  const analyser = {
    fftSize: 0,
    connect: vi.fn(),
    getByteTimeDomainData: vi.fn((samples: Uint8Array) => samples.fill(128)),
  };
  const gain = { gain: { value: 0 }, connect: vi.fn() };
  const context = {
    state: 'running',
    destination: {},
    resume: vi.fn(async () => {}),
    decodeAudioData: vi.fn(async () => ({}) as AudioBuffer),
    createGain: vi.fn(() => gain),
    createAnalyser: vi.fn(() => analyser),
    createBufferSource: vi.fn(() => source),
    close: vi.fn(async () => {}),
  };
  vi.stubGlobal('AudioContext', vi.fn(function MockAudioContext() {
    return context;
  }));
  vi.stubGlobal('fetch', vi.fn(async () => ({
    ok: true,
    status: 200,
    arrayBuffer: async () => new ArrayBuffer(0),
  })));
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  return { source };
}

function request(onStarted: () => void): PlaybackRequest {
  return {
    url: 'asset:speech.wav',
    volume: 1,
    onStarted,
    onLevel: vi.fn(),
    onEnded: vi.fn(),
  };
}

async function settlePlayback(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe('WebAudioSink', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('reports started immediately after a successful source start', async () => {
    const order: string[] = [];
    const h = audioHarness(() => order.push('source.start'));
    const sink = new WebAudioSink();

    sink.play(request(() => order.push('request.onStarted')));
    await settlePlayback();

    expect(h.source.start).toHaveBeenCalledOnce();
    expect(order).toEqual(['source.start', 'request.onStarted']);
  });

  it('does not report started when source start fails', async () => {
    audioHarness(() => { throw new Error('start failed'); });
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const onStarted = vi.fn();
    const sink = new WebAudioSink();

    sink.play(request(onStarted));
    await settlePlayback();

    expect(onStarted).not.toHaveBeenCalled();
  });

  it('keeps playback active when the started callback throws', async () => {
    const h = audioHarness(() => {});
    const callbackError = new Error('reaction failed');
    const playbackRequest = request(() => { throw callbackError; });
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const sink = new WebAudioSink();

    sink.play(playbackRequest);
    await settlePlayback();

    expect(h.source.start).toHaveBeenCalledOnce();
    expect(playbackRequest.onLevel).toHaveBeenCalled();
    expect(playbackRequest.onEnded).not.toHaveBeenCalled();
    expect(error).toHaveBeenCalledWith(
      'speech playback start callback failed',
      callbackError,
    );
  });
});
