/**
 * Web Audio implementation of [`AudioSink`]: fetch + decode + play,
 * with a per-frame RMS level for lip sync (基本設計 9章).
 */

import type {
  AudioSink,
  PlaybackHandle,
  PlaybackRequest,
} from './speech-audio-player';

/** RMS → mouth-open gain; speech RMS rarely exceeds ~0.25. */
const LEVEL_GAIN = 4;
/** Exponential smoothing factor for the mouth movement. */
const SMOOTHING = 0.5;

export class WebAudioSink implements AudioSink {
  #context: AudioContext | null = null;

  #ensureContext(): AudioContext {
    this.#context ??= new AudioContext();
    return this.#context;
  }

  play(request: PlaybackRequest): PlaybackHandle {
    let stopped = false;
    let source: AudioBufferSourceNode | null = null;
    let frame: number | null = null;

    const run = async () => {
      const context = this.#ensureContext();
      if (context.state === 'suspended') {
        await context.resume().catch(() => {});
      }
      const response = await fetch(request.url);
      if (!response.ok) {
        throw new Error(`failed to fetch wav: ${response.status}`);
      }
      const buffer = await context.decodeAudioData(
        await response.arrayBuffer(),
      );
      if (stopped) {
        return;
      }

      const gain = context.createGain();
      gain.gain.value = request.volume;
      const analyser = context.createAnalyser();
      analyser.fftSize = 512;
      source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(analyser);
      analyser.connect(gain);
      gain.connect(context.destination);

      const samples = new Uint8Array(analyser.fftSize);
      let smoothed = 0;
      const tick = () => {
        analyser.getByteTimeDomainData(samples);
        let sum = 0;
        for (const sample of samples) {
          const centered = (sample - 128) / 128;
          sum += centered * centered;
        }
        const rms = Math.sqrt(sum / samples.length);
        smoothed = smoothed * SMOOTHING + rms * (1 - SMOOTHING);
        request.onLevel(Math.min(1, smoothed * LEVEL_GAIN));
        frame = requestAnimationFrame(tick);
      };

      source.onended = () => {
        if (frame !== null) {
          cancelAnimationFrame(frame);
          frame = null;
        }
        request.onLevel(0);
        if (!stopped) {
          request.onEnded();
        }
      };
      source.start();
      tick();
    };

    run().catch((error: unknown) => {
      console.error('speech playback failed', error);
      if (frame !== null) {
        cancelAnimationFrame(frame);
        frame = null;
      }
      request.onLevel(0);
      if (!stopped) {
        request.onEnded();
      }
    });

    return {
      stop: () => {
        if (stopped) {
          return;
        }
        stopped = true;
        if (frame !== null) {
          cancelAnimationFrame(frame);
          frame = null;
        }
        try {
          source?.stop();
        } catch {
          // already stopped / never started
        }
        request.onLevel(0);
      },
    };
  }

  dispose(): void {
    const context = this.#context;
    this.#context = null;
    void context?.close().catch(() => {});
  }
}
