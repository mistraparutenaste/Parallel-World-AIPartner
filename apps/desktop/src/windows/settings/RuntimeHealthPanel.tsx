import type { RuntimeDiagnosticsDto, RuntimeHealthEventDto } from '@parallel-world/contracts';
import { RUNTIME_HEALTH_EVENT } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

const LABELS: Record<RuntimeHealthEventDto['feature'], string> = {
  audio_input: 'マイク',
  speech_to_text: '音声認識',
  language_model: 'LLM',
  text_to_speech: '音声合成',
  character_renderer: 'キャラクター表示',
};

function ownership(event: RuntimeHealthEventDto) {
  if (event.ownership && event.ownership !== 'not_applicable') return event.ownership;
  if (event.last_error?.toLowerCase().includes('owned')) return 'owned';
  if (event.last_error?.toLowerCase().includes('external')) return 'external';
  return 'runtime';
}

export function mergeHealthEvents(
  current: Record<string, RuntimeHealthEventDto>,
  incoming: RuntimeHealthEventDto[],
) {
  const merged = { ...current };
  for (const event of incoming) {
    const key = `${event.feature}:${event.ownership ?? 'not_applicable'}`;
    const existing = merged[key];
    if (!existing || event.changed_at_ms >= existing.changed_at_ms) {
      merged[key] = event;
    }
  }
  return merged;
}

export function RuntimeHealthPanel() {
  const [health, setHealth] = useState<Record<string, RuntimeHealthEventDto>>({});
  const [diagnostics, setDiagnostics] = useState<RuntimeDiagnosticsDto | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = () => void invoke<RuntimeDiagnosticsDto>('get_runtime_diagnostics')
      .then((value) => {
        if (!active || !value) return;
        setDiagnostics(value);
        setHealth((current) => mergeHealthEvents(current, value.health ?? []));
      })
      .catch(() => undefined);
    refresh();
    const poll = window.setInterval(refresh, 5_000);
    let dispose: (() => void) | undefined;
    void Promise.resolve()
      .then(() => listen<RuntimeHealthEventDto>(RUNTIME_HEALTH_EVENT, ({ payload }) => {
        if (active) setHealth((current) => mergeHealthEvents(current, [payload]));
      }))
      .then((unlisten) => {
        if (active) dispose = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      window.clearInterval(poll);
      dispose?.();
    };
  }, []);

  return (
    <section aria-labelledby="runtime-health-heading">
      <h2 id="runtime-health-heading">Runtime Health</h2>
      {Object.values(health).length === 0 ? <p>状態イベントを待機中</p> : null}
      <ul>
        {Object.values(health).map((event) => (
          <li key={`${event.feature}:${event.ownership ?? 'not_applicable'}`}>
            <strong>{LABELS[event.feature]}</strong>{' '}
            <span>{ownership(event)}</span>{' '}
            <span>{event.circuit_open ? 'circuit open' : `${event.status} / retry ${event.attempts}`}</span>
            {((event.circuit_open && ['language_model', 'text_to_speech', 'character_renderer'].includes(event.feature))
              || (event.feature === 'character_renderer' && event.status === 'recovering')) ? (
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
