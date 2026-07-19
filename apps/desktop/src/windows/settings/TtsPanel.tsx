import type {
  TtsEngineKind,
  TtsSettingsDto,
  TtsVoiceDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';

/**
 * 音声合成 section of the settings window: engine connection, voice,
 * scales and the Aivis pronunciation dictionary. Engine data is fetched
 * on demand because the selected engine may not be running.
 */
const DEFAULT_BASE_URLS: Record<TtsEngineKind, string> = {
  aivis: 'http://127.0.0.1:10101',
  irodori: 'http://127.0.0.1:8088',
};

const ENGINE_LABELS: Record<TtsEngineKind, string> = {
  aivis: 'AivisSpeech Engine',
  irodori: 'irodori-TTS',
};

export function TtsPanel() {
  const [settings, setSettings] = useState<TtsSettingsDto | null>(null);
  const [voices, setVoices] = useState<TtsVoiceDto[] | null>(null);
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

  const changeEngine = (engine: TtsEngineKind) => {
    setVoices(null);
    setSettings((current) => {
      if (!current) {
        return current;
      }
      const usesEngineDefault = Object.values(DEFAULT_BASE_URLS).includes(
        current.base_url,
      );
      return {
        ...current,
        engine,
        base_url: usesEngineDefault
          ? (DEFAULT_BASE_URLS[engine] ?? current.base_url)
          : current.base_url,
        voice_id: '',
      };
    });
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

  const fetchVoices = () => {
    setMessage(null);
    invoke<TtsVoiceDto[]>('list_tts_voices')
      .then(setVoices)
      .catch((error: unknown) => {
        const engineLabel = settings
          ? ENGINE_LABELS[settings.engine]
          : 'TTSエンジン';
        setMessage(`${engineLabel} の音声一覧を取得できません: ${String(error)}`);
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
            <label htmlFor="tts-engine">TTSエンジン</label>
            <select
              id="tts-engine"
              value={settings.engine}
              onChange={(event) =>
                changeEngine(event.target.value as TtsEngineKind)
              }
            >
              <option value="aivis">AivisSpeech Engine</option>
              <option value="irodori">irodori-TTS</option>
            </select>
          </div>
          <div>
            <label htmlFor="tts-base-url">
              接続先 ({ENGINE_LABELS[settings.engine]})
            </label>
            <input
              id="tts-base-url"
              type="text"
              value={settings.base_url}
              onChange={(event) => update({ base_url: event.target.value })}
            />
          </div>
          <div>
            <label htmlFor="tts-voice">音声</label>
            <select
              id="tts-voice"
              value={settings.voice_id}
              onChange={(event) => update({ voice_id: event.target.value })}
            >
              {(voices === null ||
                !voices.some((voice) => voice.id === settings.voice_id)) && (
                <option value={settings.voice_id}>
                  {settings.voice_id
                    ? `現在の設定 (${settings.voice_id})`
                    : '音声を選択してください'}
                </option>
              )}
              {(voices ?? []).map((voice) => (
                <option key={voice.id} value={voice.id}>
                  {voice.label}
                </option>
              ))}
            </select>
            <button type="button" onClick={fetchVoices}>
              音声一覧を取得
            </button>
          </div>
          {settings.engine === 'irodori' && (
            <div>
              <p>irodori-TTSの実行にはGPUを推奨します。</p>
              <p>初回の音声合成はモデルの読み込みに時間がかかります。</p>
              <p>本人の同意を得た参照音声のみ使用してください。</p>
            </div>
          )}
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

          {settings.engine === 'aivis' && (
            <>
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
                  {words.length === 0 && (
                    <li>登録された単語はありません</li>
                  )}
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
                  onChange={(event) =>
                    setAccentType(Number(event.target.value))
                  }
                />
                <button type="button" onClick={addWord}>
                  単語を追加
                </button>
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}
