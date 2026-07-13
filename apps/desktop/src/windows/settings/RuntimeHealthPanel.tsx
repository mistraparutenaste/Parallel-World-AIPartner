import type { RuntimeDiagnosticsDto, RuntimeHealthEventDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

const LABELS: Record<RuntimeHealthEventDto['feature'], string> = {
  audio_input: 'マイク',
  speech_to_text: '音声認識',
  language_model: 'LLM',
  text_to_speech: '音声合成',
  live2d: 'Live2D',
};

function ownership(event: RuntimeHealthEventDto) {
  if (event.ownership && event.ownership !== 'not_applicable') return event.ownership;
  if (event.last_error?.toLowerCase().includes('owned')) return 'owned';
  if (event.last_error?.toLowerCase().includes('external')) return 'external';
  return 'runtime';
}

export function RuntimeHealthPanel() {
  const [health, setHealth] = useState<Partial<Record<RuntimeHealthEventDto['feature'], RuntimeHealthEventDto>>>({});
  const [diagnostics, setDiagnostics] = useState<RuntimeDiagnosticsDto | null>(null);

  useEffect(() => {
    void invoke<RuntimeDiagnosticsDto>('get_runtime_diagnostics').then(setDiagnostics).catch(() => undefined);
    let active = true;
    let dispose: (() => void) | undefined;
    void Promise.resolve()
      .then(() => listen<RuntimeHealthEventDto>('runtime-health', ({ payload }) => {
        if (active) setHealth((current) => ({ ...current, [payload.feature]: payload }));
      }))
      .then((unlisten) => {
        if (active) dispose = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      dispose?.();
    };
  }, []);

  return (
    <section aria-labelledby="runtime-health-heading">
      <h2 id="runtime-health-heading">Runtime Health</h2>
      {Object.values(health).length === 0 ? <p>状態イベントを待機中</p> : null}
      <ul>
        {Object.values(health).map((event) => event && (
          <li key={event.feature}>
            <strong>{LABELS[event.feature]}</strong>{' '}
            <span>{ownership(event)}</span>{' '}
            <span>{event.attempts >= 8 ? 'circuit open' : `${event.status} / retry ${event.attempts}`}</span>
            {event.circuit_open && (event.ownership === 'managed' || event.feature === 'live2d') ? (
              <button
                type="button"
                onClick={() => void invoke('rearm_runtime_feature', { feature: event.feature })}
              >
                {LABELS[event.feature]} を再起動
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      <h3>Bounded queues</h3>
      <ul>
        {diagnostics?.queues.map((queue) => (
          <li key={queue.name}>{queue.name}: {queue.depth} / {queue.capacity}; dropped {queue.dropped}; busy {queue.busy}; coalesced {queue.coalesced}</li>
        ))}
      </ul>
    </section>
  );
}
