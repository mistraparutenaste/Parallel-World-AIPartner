/**
 * Ordered playback of synthesized speech (基本設計 8章).
 *
 * The player owns the queue and turn rules; actual audio output is
 * behind [`AudioSink`] so the logic is testable without Web Audio.
 */

/** One synthesized sentence, identified by turn and sequence. */
export interface SpeechAudioItem {
  turnId: number;
  seq: number;
  /** Fetchable URL of the WAV file. */
  url: string;
}

/** In-flight playback of one item. */
export interface PlaybackHandle {
  /** Halts output immediately. Idempotent. */
  stop(): void;
}

export interface PlaybackRequest {
  url: string;
  volume: number;
  /** Called repeatedly with the current output level (0..1). */
  onLevel: (level: number) => void;
  /** Called once when playback finishes or fails (not when stopped). */
  onEnded: () => void;
}

/** Audio output backend (Web Audio in production, fake in tests). */
export interface AudioSink {
  play(request: PlaybackRequest): PlaybackHandle;
  dispose?(): void;
}

export interface SpeechAudioPlayerOptions {
  /** Mirrors the audio level for lip sync (0 when silent). */
  onLevel?: (level: number) => void;
  /** Reports playback activity (STT gate / capture mute). */
  onActiveChange?: (active: boolean) => void;
}

interface PlaybackSession {
  handle: PlaybackHandle | null;
}

/**
 * Plays speech items strictly in arrival order. Items of a newer turn
 * interrupt and clear anything older; items of an older turn are
 * dropped. `stop()` halts output immediately (発話割り込み).
 */
export class SpeechAudioPlayer {
  #sink: AudioSink;
  #onLevel: (level: number) => void;
  #onActiveChange: (active: boolean) => void;
  #queue: SpeechAudioItem[] = [];
  #session: PlaybackSession | null = null;
  #currentTurn = 0;
  #active = false;
  #volume = 1;

  constructor(sink: AudioSink, options: SpeechAudioPlayerOptions = {}) {
    this.#sink = sink;
    this.#onLevel = options.onLevel ?? (() => {});
    this.#onActiveChange = options.onActiveChange ?? (() => {});
  }

  /** Master volume for subsequently started items (0..1). */
  setVolume(volume: number): void {
    this.#volume = Math.min(1, Math.max(0, volume));
  }

  /** Queues one item, applying the turn rules. */
  enqueue(item: SpeechAudioItem): void {
    if (item.turnId < this.#currentTurn) {
      return;
    }
    if (item.turnId > this.#currentTurn) {
      // A newer turn interrupts the previous one immediately.
      this.#currentTurn = item.turnId;
      this.#queue = [];
      this.#halt();
    }
    this.#queue.push(item);
    if (this.#session == null) {
      this.#playNext();
    }
  }

  /** Halts playback and clears the queue immediately. */
  stop(): void {
    this.#queue = [];
    this.#halt();
    this.#onLevel(0);
    this.#setActive(false);
  }

  /** Stops playback and releases the sink. */
  dispose(): void {
    this.stop();
    this.#sink.dispose?.();
  }

  #halt(): void {
    const session = this.#session;
    this.#session = null;
    session?.handle?.stop();
  }

  #playNext(): void {
    const item = this.#queue.shift();
    if (item === undefined) {
      this.#onLevel(0);
      this.#setActive(false);
      return;
    }
    this.#setActive(true);
    const session: PlaybackSession = { handle: null };
    this.#session = session;
    session.handle = this.#sink.play({
      url: item.url,
      volume: this.#volume,
      onLevel: (level) => {
        if (this.#session === session) {
          this.#onLevel(level);
        }
      },
      onEnded: () => {
        if (this.#session !== session) {
          return;
        }
        this.#session = null;
        this.#playNext();
      },
    });
  }

  #setActive(active: boolean): void {
    if (this.#active !== active) {
      this.#active = active;
      this.#onActiveChange(active);
    }
  }
}
