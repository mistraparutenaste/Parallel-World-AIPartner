import type {
  TtsSettingsDto,
  TtsSpeakerDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

/**
 * 音声合成 section of the settings window: AivisSpeech Engine
 * connection, voice style, scales and the pronunciation dictionary.
 * The engine list and dictionary are fetched on demand because the
 * engine may not be running.
 */
export function TtsPanel() {
  const [settings, setSettings] = useState<TtsSettingsDto | null>(null);
  const [speakers, setSpeakers] = useState<TtsSpeakerDto[] | null>(null);
  const [words, setWords] = useState<UserDictWordDto[] | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [dictMessage, setDictMessage] = useState<string | null>(null);
  const [surface, setSurface] = useState('');
  const [pronunciation, setPronunciation] = useState('');
  const [accentType, setAccentType] = useState(0);

  useEffect(() => {
    let cancelled = false;
    invoke<TtsSettingsDto>('get_tts_settings')
      .then((loaded) => {
        if (!cancelled) {
          setSettings(loaded);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setMessage(`音声合成設定を読み込めません: ${String(error)}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const update = (patch: Partial<TtsSettingsDto>) => {
    setSettings((current) => (current ? { ...current, ...patch } : current));
  };

  const save = () => {
    if (!settings) {
      return;
    }
    setMessage(null);
    invoke('set_tts_settings', { settings })
      .then(() => {
        setMessage('保存しました。次の応答から適用されます。');
      })
      .catch((error: unknown) => {
        setMessage(`保存できません: ${String(error)}`);
      });
  };

  const fetchSpeakers = () => {
    setMessage(null);
    invoke<TtsSpeakerDto[]>('list_tts_speakers')
      .then(setSpeakers)
      .catch((error: unknown) => {
        setMessage(`話者一覧を取得できません: ${String(error)}`);
      });
  };

  const loadDictionary = () => {
    setDictMessage(null);
    invoke<UserDictWordDto[]>('list_user_dict')
      .then(setWords)
      .catch((error: unknown) => {
        setDictMessage(`辞書を読み込めません: ${String(error)}`);
      });
  };

  const addWord = () => {
    setDictMessage(null);
    invoke<string>('add_user_dict_word', {
      surface,
      pronunciation,
      accentType,
    })
      .then(() => {
        setSurface('');
        setPronunciation('');
        setAccentType(0);
        loadDictionary();
      })
      .catch((error: unknown) => {
        setDictMessage(`追加できません: ${String(error)}`);
      });
  };

  const deleteWord = (uuid: string) => {
    setDictMessage(null);
    invoke('delete_user_dict_word', { uuid })
      .then(loadDictionary)
      .catch((error: unknown) => {
        setDictMessage(`削除できません: ${String(error)}`);
      });
  };

  return (
    <section aria-label="音声合成設定">
      <h2>音声合成</h2>
      {message !== null && <p role="status">{message}</p>}
      {settings !== null && (
        <>
          <div>
            <label>
              <input
                type="checkbox"
                checked={settings.enabled}
                onChange={(event) => update({ enabled: event.target.checked })}
              />
              音声で読み上げる
            </label>
          </div>
          <div>
            <label htmlFor="tts-base-url">接続先 (AivisSpeech Engine)</label>
            <input
              id="tts-base-url"
              type="text"
              value={settings.base_url}
              onChange={(event) => update({ base_url: event.target.value })}
            />
          </div>
          <div>
            <label htmlFor="tts-style">話者スタイル</label>
            <select
              id="tts-style"
              value={String(settings.style_id)}
              onChange={(event) =>
                update({ style_id: Number(event.target.value) })
              }
            >
              {(speakers === null ||
                !speakers.some(
                  (speaker) => speaker.style_id === settings.style_id,
                )) && (
                <option value={String(settings.style_id)}>
                  現在の設定 ({settings.style_id})
                </option>
              )}
              {(speakers ?? []).map((speaker) => (
                <option key={speaker.style_id} value={String(speaker.style_id)}>
                  {speaker.name} / {speaker.style_name}
                </option>
              ))}
            </select>
            <button type="button" onClick={fetchSpeakers}>
              話者一覧を取得
            </button>
          </div>
          <div>
            <label htmlFor="tts-volume">音量 ({settings.volume.toFixed(1)})</label>
            <input
              id="tts-volume"
              type="range"
              min="0"
              max="2"
              step="0.1"
              value={settings.volume}
              onChange={(event) =>
                update({ volume: Number(event.target.value) })
              }
            />
          </div>
          <div>
            <label htmlFor="tts-speed">話速 ({settings.speed.toFixed(1)})</label>
            <input
              id="tts-speed"
              type="range"
              min="0.5"
              max="2"
              step="0.1"
              value={settings.speed}
              onChange={(event) => update({ speed: Number(event.target.value) })}
            />
          </div>
          <button type="button" onClick={save}>
            保存
          </button>

          <h3>ユーザー辞書</h3>
          {dictMessage !== null && <p role="status">{dictMessage}</p>}
          <button type="button" onClick={loadDictionary}>
            辞書を読み込む
          </button>
          {words !== null && (
            <ul aria-label="登録済みの単語">
              {words.map((word) => (
                <li key={word.uuid}>
                  {word.surface}（{word.pronunciation}）
                  <button
                    type="button"
                    onClick={() => deleteWord(word.uuid)}
                    aria-label={`${word.surface}を削除`}
                  >
                    削除
                  </button>
                </li>
              ))}
              {words.length === 0 && <li>登録された単語はありません</li>}
            </ul>
          )}
          <div>
            <label htmlFor="dict-surface">単語</label>
            <input
              id="dict-surface"
              type="text"
              value={surface}
              onChange={(event) => setSurface(event.target.value)}
            />
            <label htmlFor="dict-pronunciation">読み（カタカナ）</label>
            <input
              id="dict-pronunciation"
              type="text"
              value={pronunciation}
              onChange={(event) => setPronunciation(event.target.value)}
            />
            <label htmlFor="dict-accent">アクセント核位置</label>
            <input
              id="dict-accent"
              type="number"
              min="0"
              value={accentType}
              onChange={(event) => setAccentType(Number(event.target.value))}
            />
            <button type="button" onClick={addWord}>
              単語を追加
            </button>
          </div>
        </>
      )}
    </section>
  );
}
