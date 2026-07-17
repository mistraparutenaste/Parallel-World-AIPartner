import type { DataDeletionResultDto, DataUsageDto } from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';

type DangerActionId = 'history' | 'memory' | 'tts';
type Operation = 'export' | DangerActionId;
type Feedback = { kind: 'status' | 'error'; text: string };
type UsageState = 'loading' | 'ready' | 'error';

const numberFormatter = new Intl.NumberFormat('ja-JP');
const byteFormatter = new Intl.NumberFormat('ja-JP', { maximumFractionDigits: 1 });

function formatCount(value: number | undefined, unit: string): string {
  return value === undefined ? '確認中…' : `${numberFormatter.format(value)}${unit}`;
}

function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined) return '確認中…';
  if (bytes < 1024) return `${numberFormatter.format(bytes)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${byteFormatter.format(value)} ${units[index]}`;
}

function DangerAction({
  title,
  description,
  usage,
  buttonLabel,
  phrase,
  busy,
  disabled,
  confirming,
  confirmationText,
  triggerRef,
  onBegin,
  onConfirmationChange,
  onCancel,
  onConfirm,
}: {
  title: string;
  description: string;
  usage: string;
  buttonLabel: string;
  phrase: string;
  busy: boolean;
  disabled: boolean;
  confirming: boolean;
  confirmationText: string;
  triggerRef: RefObject<HTMLButtonElement | null>;
  onBegin: () => void;
  onConfirmationChange: (value: string) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="danger-action">
      <div className="danger-action__description">
        <h3>{title}</h3>
        <p>{description}</p>
        <p className="danger-action__usage">現在: {usage}</p>
      </div>
      <button
        ref={triggerRef}
        type="button"
        className="danger-button"
        disabled={disabled}
        onClick={onBegin}
      >
        {buttonLabel}
      </button>

      {confirming ? (
        <div className="danger-confirmation" role="group" aria-label={`${title}の最終確認`}>
          <p>
            この操作は取り消せません。続行するには
            <strong>「{phrase}」</strong>
            と入力してください。
          </p>
          <label>
            <span>確認用テキスト</span>
            <input
              autoFocus
              value={confirmationText}
              disabled={busy}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => onConfirmationChange(event.target.value)}
            />
          </label>
          <div className="danger-confirmation__actions">
            <button type="button" className="secondary-button" disabled={busy} onClick={onCancel}>
              キャンセル
            </button>
            <button
              type="button"
              className="danger-button danger-button--confirm"
              disabled={disabled || confirmationText !== phrase}
              onClick={onConfirm}
            >
              {busy ? '削除中…' : '完全に削除する'}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function DataPanel() {
  const [destination, setDestination] = useState('');
  const [exportFeedback, setExportFeedback] = useState<Feedback | null>(null);
  const [dangerFeedback, setDangerFeedback] = useState<Feedback | null>(null);
  const [operation, setOperation] = useState<Operation | null>(null);
  const [usage, setUsage] = useState<DataUsageDto | null>(null);
  const [usageState, setUsageState] = useState<UsageState>('loading');
  const [confirming, setConfirming] = useState<DangerActionId | null>(null);
  const [confirmationText, setConfirmationText] = useState('');
  const historyTriggerRef = useRef<HTMLButtonElement>(null);
  const memoryTriggerRef = useRef<HTMLButtonElement>(null);
  const ttsTriggerRef = useRef<HTMLButtonElement>(null);
  const usageRetryRef = useRef<HTMLButtonElement>(null);
  const pendingFocusRef = useRef<DangerActionId | null>(null);
  const usageRequestGenerationRef = useRef(0);
  const mountedRef = useRef(false);
  const busy = operation !== null;
  const usageReady = usageState === 'ready' && usage !== null;

  const refreshUsage = useCallback(async () => {
    if (!mountedRef.current) return;
    const requestGeneration = ++usageRequestGenerationRef.current;
    setUsage(null);
    setUsageState('loading');
    try {
      const nextUsage = await invoke<DataUsageDto>('get_data_usage');
      if (!mountedRef.current || requestGeneration !== usageRequestGenerationRef.current) return;
      setUsage(nextUsage);
      setUsageState('ready');
    } catch {
      if (!mountedRef.current || requestGeneration !== usageRequestGenerationRef.current) return;
      setUsageState('error');
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void refreshUsage();
    return () => {
      mountedRef.current = false;
      usageRequestGenerationRef.current += 1;
    };
  }, [refreshUsage]);

  useEffect(() => {
    const action = pendingFocusRef.current;
    if (action === null || busy || confirming !== null) return;
    if (usageReady) {
      const trigger = action === 'history'
        ? historyTriggerRef.current
        : action === 'memory'
          ? memoryTriggerRef.current
          : ttsTriggerRef.current;
      trigger?.focus();
      pendingFocusRef.current = null;
      return;
    }
    if (usageState === 'error') {
      usageRetryRef.current?.focus();
      pendingFocusRef.current = null;
    }
  }, [busy, confirming, usageReady, usageState]);

  const openConfirmation = (action: DangerActionId) => {
    setConfirming(action);
    setConfirmationText('');
    setDangerFeedback(null);
  };

  const closeConfirmation = () => {
    pendingFocusRef.current = confirming;
    setConfirming(null);
    setConfirmationText('');
  };

  const runDestructive = async (action: DangerActionId, command: string) => {
    setOperation(action);
    setDangerFeedback(null);
    try {
      const result = await invoke<DataDeletionResultDto | null>(command);
      const text = action === 'history'
        ? '会話履歴と要約を削除しました。'
        : action === 'memory'
          ? `${formatCount(result?.deleted_records, '件')}の記憶データを削除しました。`
          : `${formatCount(result?.deleted_files, '件')}の音声ファイル（${formatBytes(result?.freed_bytes)}）を削除しました。`;
      setDangerFeedback({ kind: 'status', text });
      closeConfirmation();
    } catch (error) {
      setDangerFeedback({ kind: 'error', text: String(error) });
    } finally {
      await refreshUsage();
      setOperation(null);
    }
  };

  const exportData = async () => {
    setOperation('export');
    setExportFeedback(null);
    const args = { destination: destination.trim(), allowOverwrite: false };
    try {
      await invoke('export_user_data', args);
      setExportFeedback({ kind: 'status', text: 'データをエクスポートしました。' });
    } catch (error) {
      if (
        String(error).includes('DESTINATION_EXISTS')
        && window.confirm('保存先は既に存在します。上書きしますか？')
      ) {
        try {
          await invoke('export_user_data', { ...args, allowOverwrite: true });
          setExportFeedback({ kind: 'status', text: 'データを上書きエクスポートしました。' });
          return;
        } catch (overwriteError) {
          setExportFeedback({ kind: 'error', text: String(overwriteError) });
          return;
        }
      }
      setExportFeedback({ kind: 'error', text: String(error) });
    } finally {
      setOperation(null);
    }
  };

  return (
    <>
      <section aria-labelledby="data-title" aria-busy={operation === 'export'}>
        <h2 id="data-title">データ</h2>
        <p>会話履歴、要約、長期記憶をバックアップできます。</p>
        <div className="data-export-row">
          <label htmlFor="export-path">
            <span>保存先</span>
            <input
              id="export-path"
              disabled={busy}
              value={destination}
              onChange={(event) => setDestination(event.target.value)}
            />
          </label>
          <button
            type="button"
            disabled={busy || !destination.trim()}
            onClick={() => void exportData()}
          >
            {operation === 'export' ? 'エクスポート中…' : 'エクスポート'}
          </button>
        </div>
        {exportFeedback?.kind === 'status' ? <p role="status">{exportFeedback.text}</p> : null}
        {exportFeedback?.kind === 'error' ? <p role="alert">{exportFeedback.text}</p> : null}
      </section>

      <section
        className="danger-zone"
        aria-labelledby="danger-zone-title"
        aria-busy={busy || usageState === 'loading'}
      >
        <h2 id="danger-zone-title">デンジャーゾーン</h2>
        <p className="danger-zone__intro">
          ここで削除したデータは元に戻せません。必要であれば先にエクスポートしてください。
        </p>
        {usageState === 'loading' ? (
          <p className="danger-zone__usage-status" role="status">
            現在の使用量を確認しています…
          </p>
        ) : null}
        {usageState === 'error' ? (
          <div className="danger-zone__usage-error">
            <p role="alert">現在の使用量を取得できませんでした。削除操作は無効です。</p>
            <button
              ref={usageRetryRef}
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={() => void refreshUsage()}
            >
              使用量を再取得
            </button>
          </div>
        ) : null}
        {dangerFeedback?.kind === 'status' ? (
          <p className="danger-zone__feedback" role="status">{dangerFeedback.text}</p>
        ) : null}
        {dangerFeedback?.kind === 'error' ? (
          <p className="danger-zone__feedback" role="alert">{dangerFeedback.text}</p>
        ) : null}
        <div className="danger-action-list">
          <DangerAction
            title="会話履歴と要約"
            description="保存された会話メッセージと会話要約を削除します。長期記憶は残ります。"
            usage={`${formatCount(usage?.conversation_messages, '件のメッセージ')} / ${formatCount(usage?.conversation_summaries, '件の要約')}`}
            buttonLabel="履歴を削除"
            phrase="履歴を削除"
            busy={busy}
            disabled={busy || !usageReady}
            confirming={confirming === 'history'}
            confirmationText={confirmationText}
            triggerRef={historyTriggerRef}
            onBegin={() => openConfirmation('history')}
            onConfirmationChange={setConfirmationText}
            onCancel={closeConfirmation}
            onConfirm={() => void runDestructive('history', 'delete_conversation_history')}
          />
          <DangerAction
            title="要約と長期記憶"
            description="会話要約と学習した長期記憶を削除します。会話履歴と性格設定は残ります。"
            usage={`${formatCount(usage?.conversation_summaries, '件の要約')} / ${formatCount(usage?.long_term_memories, '件の長期記憶')}`}
            buttonLabel="記憶を削除"
            phrase="記憶を削除"
            busy={busy}
            disabled={busy || !usageReady}
            confirming={confirming === 'memory'}
            confirmationText={confirmationText}
            triggerRef={memoryTriggerRef}
            onBegin={() => openConfirmation('memory')}
            onConfirmationChange={setConfirmationText}
            onCancel={closeConfirmation}
            onConfirm={() => void runDestructive('memory', 'delete_memories')}
          />
          <DangerAction
            title="TTS音声キャッシュ"
            description="AivisSpeechで生成したWAVキャッシュだけを削除します。音声モデルと話者設定は残ります。"
            usage={`${formatCount(usage?.tts_audio_files, 'ファイル')} / ${formatBytes(usage?.tts_audio_bytes)}`}
            buttonLabel="音声キャッシュを削除"
            phrase="音声を削除"
            busy={busy}
            disabled={busy || !usageReady}
            confirming={confirming === 'tts'}
            confirmationText={confirmationText}
            triggerRef={ttsTriggerRef}
            onBegin={() => openConfirmation('tts')}
            onConfirmationChange={setConfirmationText}
            onCancel={closeConfirmation}
            onConfirm={() => void runDestructive('tts', 'clear_tts_audio_cache')}
          />
        </div>
      </section>
    </>
  );
}
