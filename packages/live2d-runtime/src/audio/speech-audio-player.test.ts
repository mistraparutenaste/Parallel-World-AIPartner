import { describe, expect, it, vi } from 'vitest';
import type { AudioSink, PlaybackRequest } from './speech-audio-player';
import { SpeechAudioPlayer } from './speech-audio-player';

/** Records play requests; playback ends only when the test says so. */
class FakeSink implements AudioSink {
  requests: Array<PlaybackRequest & { stopped: boolean }> = [];
  disposed = false;

  play(request: PlaybackRequest) {
    const entry = { ...request, stopped: false };
    this.requests.push(entry);
    return {
      stop: () => {
        entry.stopped = true;
      },
    };
  }

  dispose() {
    this.disposed = true;
  }

  endCurrent(index: number) {
    this.requests[index]?.onEnded();
  }

  startCurrent(index: number) {
    this.requests[index]?.onStarted();
  }
}

function urls(sink: FakeSink): string[] {
  return sink.requests.map((request) => request.url);
}

describe('SpeechAudioPlayer', () => {
  it('plays items of one turn strictly in order', () => {
    const sink = new FakeSink();
    const player = new SpeechAudioPlayer(sink);

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    player.enqueue({ turnId: 1, seq: 1, url: 'b.wav' });
    expect(urls(sink)).toEqual(['a.wav']);

    sink.endCurrent(0);
    expect(urls(sink)).toEqual(['a.wav', 'b.wav']);
  });

  it('reports activity around playback for the stt gate', () => {
    const sink = new FakeSink();
    const active = vi.fn();
    const player = new SpeechAudioPlayer(sink, { onActiveChange: active });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    expect(active).toHaveBeenLastCalledWith(true);

    sink.endCurrent(0);
    expect(active).toHaveBeenLastCalledWith(false);
    expect(active).toHaveBeenCalledTimes(2);
  });

  it('forwards audio levels only from the current item', () => {
    const sink = new FakeSink();
    const levels: number[] = [];
    const player = new SpeechAudioPlayer(sink, {
      onLevel: (level) => levels.push(level),
    });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    sink.requests[0].onLevel(0.5);
    player.stop();
    sink.requests[0].onLevel(0.9); // stale callback after stop

    expect(levels).toEqual([0.5, 0]);
  });

  it('a newer turn interrupts and clears the older one', () => {
    const sink = new FakeSink();
    const player = new SpeechAudioPlayer(sink);

    player.enqueue({ turnId: 1, seq: 0, url: 'old-0.wav' });
    player.enqueue({ turnId: 1, seq: 1, url: 'old-1.wav' });
    player.enqueue({ turnId: 2, seq: 0, url: 'new-0.wav' });

    expect(sink.requests[0].stopped).toBe(true);
    expect(urls(sink)).toEqual(['old-0.wav', 'new-0.wav']);

    // The dequeued old item never plays, even after the new one ends.
    sink.endCurrent(1);
    expect(urls(sink)).toEqual(['old-0.wav', 'new-0.wav']);
  });

  it('drops late items of an older turn', () => {
    const sink = new FakeSink();
    const player = new SpeechAudioPlayer(sink);

    player.enqueue({ turnId: 2, seq: 0, url: 'new.wav' });
    player.enqueue({ turnId: 1, seq: 5, url: 'stale.wav' });
    sink.endCurrent(0);

    expect(urls(sink)).toEqual(['new.wav']);
  });

  it('stop halts immediately and later turns can speak again', () => {
    const sink = new FakeSink();
    const active = vi.fn();
    const player = new SpeechAudioPlayer(sink, { onActiveChange: active });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    player.enqueue({ turnId: 1, seq: 1, url: 'b.wav' });
    player.stop();

    expect(sink.requests[0].stopped).toBe(true);
    expect(active).toHaveBeenLastCalledWith(false);
    expect(urls(sink)).toEqual(['a.wav']);

    player.enqueue({ turnId: 2, seq: 0, url: 'c.wav' });
    expect(urls(sink)).toEqual(['a.wav', 'c.wav']);
  });

  it('passes the master volume to the sink', () => {
    const sink = new FakeSink();
    const player = new SpeechAudioPlayer(sink);
    player.setVolume(0.25);

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    expect(sink.requests[0].volume).toBe(0.25);
  });

  it('reports a turn only after playback actually starts', () => {
    const sink = new FakeSink();
    const started = vi.fn();
    const player = new SpeechAudioPlayer(sink, {
      onTurnPlaybackStart: started,
    });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    expect(started).not.toHaveBeenCalled();

    sink.startCurrent(0);
    expect(started).toHaveBeenCalledOnce();
    expect(started).toHaveBeenLastCalledWith(1);
  });

  it('reports at most once per turn across sequential chunks and once for a new turn', () => {
    const sink = new FakeSink();
    const started = vi.fn();
    const player = new SpeechAudioPlayer(sink, {
      onTurnPlaybackStart: started,
    });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    player.enqueue({ turnId: 1, seq: 1, url: 'b.wav' });
    sink.startCurrent(0);
    sink.endCurrent(0);
    sink.startCurrent(1);
    player.enqueue({ turnId: 2, seq: 0, url: 'c.wav' });
    sink.startCurrent(2);

    expect(started.mock.calls).toEqual([[1], [2]]);
  });

  it('ignores stale start callbacks after stop and dispose', () => {
    const sink = new FakeSink();
    const started = vi.fn();
    const player = new SpeechAudioPlayer(sink, {
      onTurnPlaybackStart: started,
    });

    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });
    player.stop();
    sink.startCurrent(0);

    player.enqueue({ turnId: 2, seq: 0, url: 'b.wav' });
    player.dispose();
    sink.startCurrent(1);

    expect(started).not.toHaveBeenCalled();
  });

  it('does not report a failed playback that ends before starting', () => {
    const sink = new FakeSink();
    const started = vi.fn();
    const player = new SpeechAudioPlayer(sink, {
      onTurnPlaybackStart: started,
    });

    player.enqueue({ turnId: 1, seq: 0, url: 'failed.wav' });
    sink.endCurrent(0);

    expect(started).not.toHaveBeenCalled();
  });

  it('dispose stops playback and releases the sink', () => {
    const sink = new FakeSink();
    const player = new SpeechAudioPlayer(sink);
    player.enqueue({ turnId: 1, seq: 0, url: 'a.wav' });

    player.dispose();

    expect(sink.requests[0].stopped).toBe(true);
    expect(sink.disposed).toBe(true);
  });
});
