import type {
  AudioDeviceDto,
  AudioDiagnosticsDto,
  AudioLevelEventDto,
  SttStateEventDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';

const PHASE_LABELS: Record<SttStateEventDto['phase'], string> = {
  starting: '起動中',
  listening: '聞き取り中',
  stopped: '停止',
  unavailable: '利用できません',
};

/**
 * Microphone section of the settings window: device selection,
 * start/stop, mute, input level and pipeline diagnostics.
 */
export function MicrophonePanel() {
  const [devices, setDevices] = useState<AudioDeviceDto[]>([]);
  const [selectedDevice, setSelectedDevice] = useState<string>('');
  const [phase, setPhase] = useState<SttStateEventDto['phase']>('stopped');
  const [message, setMessage] = useState<string | null>(null);
  const [level, setLevel] = useState(0);
  const [muted, setMuted] = useState(false);
  const [diagnostics, setDiagnostics] = useState<AudioDiagnosticsDto | null>(
    null,
  );
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    invoke<AudioDeviceDto[]>('list_microphones')
      .then((list) => {
        if (!disposed) {
          setDevices(list);
          setSelectedDevice(list.find((d) => d.is_default)?.id ?? '');
        }
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setMessage(`マイクを列挙できません: ${String(error)}`);
        }
      });

    const subscribe = async () => {
      const stopState = await listen<SttStateEventDto>('stt-state', (event) => {
        setPhase(event.payload.phase);
        setMessage(event.payload.message ?? null);
      });
      const stopLevel = await listen<AudioLevelEventDto>(
        'stt-level',
        (event) => {
          setLevel(event.payload.rms);
        },
      );
      if (disposed) {
        stopState();
        stopLevel();
      } else {
        unlisteners.push(stopState, stopLevel);
      }
    };
    void subscribe();

    const timer = setInterval(() => {
      if (phaseRef.current === 'listening') {
        invoke<AudioDiagnosticsDto>('get_audio_diagnostics')
          .then((snapshot) => {
            if (!disposed) {
              setDiagnostics(snapshot);
            }
          })
          .catch(() => {});
      }
    }, 2000);

    return () => {
      disposed = true;
      clearInterval(timer);
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, []);

  const start = () => {
    setMessage(null);
    invoke('start_listening', {
      deviceId: selectedDevice === '' ? null : selectedDevice,
    }).catch((error: unknown) => {
      setMessage(`音声認識を開始できません: ${String(error)}`);
    });
  };

  const stop = () => {
    invoke('stop_listening').catch(() => {});
  };

  const toggleMute = (nextMuted: boolean) => {
    setMuted(nextMuted);
    invoke('set_capture_enabled', { enabled: !nextMuted }).catch(() => {});
  };

  return (
    <section aria-label="マイク設定">
      <h2>マイク</h2>
      {message !== null && <p role="alert">{message}</p>}
      <div>
        <label htmlFor="microphone-select">入力デバイス</label>
        <select
          id="microphone-select"
          value={selectedDevice}
          onChange={(event) => setSelectedDevice(event.target.value)}
        >
          {devices.map((device) => (
            <option key={device.id} value={device.id}>
              {device.name}
              {device.is_default ? '（既定）' : ''}
            </option>
          ))}
        </select>
      </div>
      <p>
        状態: <output>{PHASE_LABELS[phase]}</output>
      </p>
      <div>
        <button type="button" onClick={start} disabled={phase === 'listening'}>
          聞き取り開始
        </button>
        <button type="button" onClick={stop} disabled={phase === 'stopped'}>
          聞き取り停止
        </button>
        <label>
          <input
            type="checkbox"
            checked={muted}
            onChange={(event) => toggleMute(event.target.checked)}
          />
          ミュート
        </label>
      </div>
      <div>
        <label htmlFor="input-level">入力レベル</label>
        <meter id="input-level" min={0} max={0.5} value={level} />
      </div>
      {diagnostics !== null && (
        <details open>
          <summary>診断</summary>
          <ul>
            <li>処理フレーム数: {diagnostics.frames_processed}</li>
            <li>確定セグメント数: {diagnostics.segments_completed}</li>
            <li>採用: {diagnostics.transcripts_accepted}</li>
            <li>棄却: {diagnostics.transcripts_rejected}</li>
            <li>ドロップサンプル数: {diagnostics.dropped_samples}</li>
          </ul>
        </details>
      )}
    </section>
  );
}
