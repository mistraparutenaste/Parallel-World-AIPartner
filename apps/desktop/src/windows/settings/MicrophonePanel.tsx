import type {
  AudioDeviceDto,
  AudioDiagnosticsDto,
  AudioLevelEventDto,
  DeviceFallbackEventDto,
  RuntimeHealthEventDto,
  SttStateEventDto,
} from '@parallel-world/contracts';
import {
  RUNTIME_HEALTH_EVENT,
  STT_DEVICE_FALLBACK_EVENT,
  STT_LEVEL_EVENT,
  STT_STATE_EVENT,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { subscribeEvent } from '../../shared/ipc/event-bus';

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
  const [fallbackMessage, setFallbackMessage] = useState<string | null>(null);
  const [phase, setPhase] = useState<SttStateEventDto['phase']>('stopped');
  const [message, setMessage] = useState<string | null>(null);
  const [level, setLevel] = useState(0);
  const [muted, setMuted] = useState(false);
  const [diagnostics, setDiagnostics] = useState<AudioDiagnosticsDto | null>(
    null,
  );
  const phaseRef = useRef(phase);
  const preferredDeviceRef = useRef(selectedDevice);
  phaseRef.current = phase;
  preferredDeviceRef.current = selectedDevice;

  useEffect(() => {
    let disposed = false;
    let stateEventSeen = false;

    const refreshDevices = () => invoke<AudioDeviceDto[]>('list_microphones')
      .then((list) => {
        if (!disposed) {
          setDevices(list);
          const preferred = preferredDeviceRef.current;
          if (preferred === '') {
            const initial = list.find((d) => d.is_default)?.id ?? '';
            preferredDeviceRef.current = initial;
            setSelectedDevice(initial);
          } else if (list.some((device) => device.id === preferred)) {
            setFallbackMessage(null);
          }
        }
      })
      .catch((error: unknown) => {
        if (!disposed) {
          setMessage(`マイクを列挙できません: ${String(error)}`);
        }
      });
    void refreshDevices();

    const stopState = subscribeEvent<SttStateEventDto>(STT_STATE_EVENT, (payload) => {
      stateEventSeen = true;
      setPhase(payload.phase);
      setMessage(payload.message ?? null);
    });
    invoke<SttStateEventDto>('get_stt_state')
      .then((snapshot) => {
        if (!disposed && !stateEventSeen) {
          setPhase(snapshot.phase);
          setMessage(snapshot.message ?? null);
        }
      })
      .catch(() => {});
    const stopLevel = subscribeEvent<AudioLevelEventDto>(STT_LEVEL_EVENT, (payload) => {
      setLevel(payload.rms);
    });
    const stopHealth = subscribeEvent<RuntimeHealthEventDto>(RUNTIME_HEALTH_EVENT, (payload) => {
      if (payload.feature !== 'audio_input') return;
      if (payload.status === 'recovering') {
        if (payload.circuit_open) return;
        setMessage('マイクを再接続しています…');
        void refreshDevices();
      } else if (payload.status === 'degraded') {
        setMessage(payload.last_error ?? '選択したマイクが見つからないため既定のマイクを使用します。');
        void refreshDevices();
      } else if (payload.status === 'healthy') {
        setMessage(null);
      }
    });
    const stopFallback = subscribeEvent<DeviceFallbackEventDto>(STT_DEVICE_FALLBACK_EVENT, (payload) => {
      setFallbackMessage(`選択したマイクが見つからないため、既定のマイク（${payload.active_device_id ?? '自動選択'}）を使用しています。`);
      void refreshDevices();
    });

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
      stopState();
      stopLevel();
      stopHealth();
      stopFallback();
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
      {(fallbackMessage ?? message) !== null && <p role="alert">{fallbackMessage ?? message}</p>}
      <div>
        <label htmlFor="microphone-select">入力デバイス</label>
        <select
          id="microphone-select"
          value={selectedDevice}
          onChange={(event) => {
            const nextDevice = event.target.value;
            preferredDeviceRef.current = nextDevice;
            setSelectedDevice(nextDevice);
            setFallbackMessage(null);
            if (phaseRef.current !== 'stopped') {
              void invoke('set_input_device', {
                deviceId: nextDevice === '' ? null : nextDevice,
              }).catch((error: unknown) => {
                setMessage(`入力デバイスを切り替えられません: ${String(error)}`);
              });
            }
          }}
        >
          {selectedDevice !== '' && !devices.some((device) => device.id === selectedDevice) && (
            <option value={selectedDevice}>選択したデバイス（接続待ち）</option>
          )}
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
            <li>障害通知キュー: {diagnostics.failure_queue_depth}</li>
            <li>破棄された障害通知: {diagnostics.failure_queue_dropped}</li>
          </ul>
        </details>
      )}
    </section>
  );
}
