import type {
  IrodoriInstallStateDto,
  TtsEngineKind,
  TtsSettingsDto,
  TtsVoiceDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useEffect, useRef, useState } from 'react';

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

/**
 * Irodori expects the adapter directory, so a picked adapter file such as
 * `adapter_model.safetensors` resolves to the directory that contains it.
 * Both separators are handled because the settings value may come from a
 * Windows host.
 */
function parentDirectory(filePath: string): string {
  const index = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\'));
  if (index < 0) {
    return filePath;
  }
  if (index === 0 || filePath[index - 1] === ':') {
    return filePath.slice(0, index + 1);
  }
  return filePath.slice(0, index);
}

export function TtsPanel() {
  const [settings, setSettings] = useState<TtsSettingsDto | null>(null);
  const settingsRef = useRef<TtsSettingsDto | null>(null);
  const voiceRequestGenerationRef = useRef(0);
  const [voices, setVoices] = useState<TtsVoiceDto[] | null>(null);
  const [words, setWords] = useState<UserDictWordDto[] | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [dictMessage, setDictMessage] = useState<string | null>(null);
  const [surface, setSurface] = useState('');
  const [pronunciation, setPronunciation] = useState('');
  const [accentType, setAccentType] = useState(0);
  const [irodoriInstall, setIrodoriInstall] = useState<IrodoriInstallStateDto | null>(null);
  const [installingIrodori, setInstallingIrodori] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<TtsSettingsDto>('get_tts_settings')
      .then((loaded) => {
        if (!cancelled) {
          settingsRef.current = loaded;
          setSettings(loaded);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setMessage(`音声合成設定を読み込めません: ${String(error)}`);
        }
      });
    invoke<IrodoriInstallStateDto>('get_irodori_install_state')
      .then((state) => {
        if (!cancelled) setIrodoriInstall(state);
      })
      .catch(() => {
        if (!cancelled) setIrodoriInstall(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const installIrodori = async () => {
    setInstallingIrodori(true);
    try {
      await invoke('install_irodori');
      setMessage('Irodoriインストーラーを別ウィンドウで起動しました。');
    } catch (error) {
      setMessage(`Irodoriインストーラーを起動できません: ${String(error)}`);
    } finally {
      setInstallingIrodori(false);
    }
  };

  const browseLoraAdapter = async (mode: 'directory' | 'file') => {
    setMessage(null);
    const current = settingsRef.current?.irodori_lora_adapter ?? '';
    const defaultPath = current === '' ? {} : { defaultPath: current };
    try {
      const selected = await open(
        mode === 'directory'
          ? { directory: true, multiple: false, ...defaultPath }
          : {
              multiple: false,
              filters: [
                {
                  name: 'LoRA adapter',
                  extensions: ['safetensors', 'bin', 'pt', 'json'],
                },
                { name: 'すべてのファイル', extensions: ['*'] },
              ],
              ...defaultPath,
            },
      );
      if (typeof selected !== 'string') {
        return;
      }
      update({
        irodori_lora_adapter:
          mode === 'directory' ? selected : parentDirectory(selected),
      });
    } catch (error) {
      setMessage(
        `${mode === 'directory' ? 'フォルダ' : 'ファイル'}を選択できません: ${String(error)}`,
      );
    }
  };

  const update = (patch: Partial<TtsSettingsDto>) => {
    if (patch.engine !== undefined || patch.base_url !== undefined) {
      voiceRequestGenerationRef.current += 1;
      setVoices(null);
    }
    setSettings((current) => {
      if (!current) {
        return current;
      }
      const next = { ...current, ...patch };
      settingsRef.current = next;
      return next;
    });
  };

  const changeEngine = (engine: TtsEngineKind) => {
    voiceRequestGenerationRef.current += 1;
    setVoices(null);
    setSettings((current) => {
      if (!current) {
        return current;
      }
      const usesEngineDefault = Object.values(DEFAULT_BASE_URLS).includes(
        current.base_url,
      );
      const next = {
        ...current,
        engine,
        base_url: usesEngineDefault
          ? (DEFAULT_BASE_URLS[engine] ?? current.base_url)
          : current.base_url,
        voice_id: '',
      };
      settingsRef.current = next;
      return next;
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
    if (!settings) {
      return;
    }
    const request = {
      engine: settings.engine,
      baseUrl: settings.base_url,
    };
    const generation = voiceRequestGenerationRef.current + 1;
    voiceRequestGenerationRef.current = generation;
    setMessage(null);
    invoke<TtsVoiceDto[]>('list_tts_voices', {
      engine: request.engine,
      baseUrl: request.baseUrl,
    })
      .then((loadedVoices) => {
        const current = settingsRef.current;
        if (
          generation === voiceRequestGenerationRef.current &&
          current?.engine === request.engine &&
          current.base_url === request.baseUrl
        ) {
          setVoices(loadedVoices);
        }
      })
      .catch((error: unknown) => {
        const current = settingsRef.current;
        if (
          generation !== voiceRequestGenerationRef.current ||
          current?.engine !== request.engine ||
          current.base_url !== request.baseUrl
        ) {
          return;
        }
        const engineLabel = ENGINE_LABELS[request.engine];
        setMessage(`${engineLabel} の音声一覧を取得できません: ${String(error)}`);
      });
  };

  type DictionaryTarget = Pick<TtsSettingsDto, 'engine'> & {
    baseUrl: string;
  };

  const currentDictionaryTarget = (): DictionaryTarget | null => {
    const current = settingsRef.current;
    return current
      ? { engine: current.engine, baseUrl: current.base_url }
      : null;
  };

  const loadDictionary = (target = currentDictionaryTarget()) => {
    if (!target) {
      return;
    }
    setDictMessage(null);
    invoke<UserDictWordDto[]>('list_user_dict', target)
      .then(setWords)
      .catch((error: unknown) => {
        setDictMessage(`辞書を読み込めません: ${String(error)}`);
      });
  };

  const addWord = () => {
    const target = currentDictionaryTarget();
    if (!target) {
      return;
    }
    setDictMessage(null);
    invoke<string>('add_user_dict_word', {
      ...target,
      surface,
      pronunciation,
      accentType,
    })
      .then(() => {
        setSurface('');
        setPronunciation('');
        setAccentType(0);
        loadDictionary(target);
      })
      .catch((error: unknown) => {
        setDictMessage(`追加できません: ${String(error)}`);
      });
  };

  const deleteWord = (uuid: string) => {
    const target = currentDictionaryTarget();
    if (!target) {
      return;
    }
    setDictMessage(null);
    invoke('delete_user_dict_word', { ...target, uuid })
      .then(() => loadDictionary(target))
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
              {irodoriInstall && !irodoriInstall.installed ? (
                <div className="setting-row">
                  <div>
                    <strong>Irodoriがまだ導入されていません</strong>
                    <p>約2.5GBのモデルを含むため、別ウィンドウで確認してからダウンロードします。</p>
                  </div>
                  <button
                    type="button"
                    disabled={installingIrodori}
                    onClick={() => void installIrodori()}
                  >
                    {installingIrodori ? '起動中…' : 'インストールを開始'}
                  </button>
                </div>
              ) : null}
              {irodoriInstall?.installed ? <p>管理対象のIrodoriは導入済みです。</p> : null}
              <div>
                <label htmlFor="tts-irodori-lora">LoRA adapter path</label>
                <input
                  id="tts-irodori-lora"
                  type="text"
                  value={settings.irodori_lora_adapter}
                  placeholder="C:/models/adapters/character-a"
                  onChange={(event) =>
                    update({ irodori_lora_adapter: event.target.value })
                  }
                />
                <button
                  type="button"
                  onClick={() => void browseLoraAdapter('directory')}
                >
                  フォルダを選択
                </button>
                <button
                  type="button"
                  onClick={() => void browseLoraAdapter('file')}
                >
                  ファイルから選択
                </button>
              </div>
              <p>
                未指定時はbase modelを使用します。Irodoriサーバーから参照できるadapterディレクトリを指定してください。
              </p>
              <p>
                「ファイルから選択」では adapter_model.safetensors などを選ぶと、そのファイルがあるフォルダを設定します。
              </p>
              <p>
            Dynamic LoRA使用時はサーバーでIRODORI_COMPILE_MODEL=falseにしてください。
              </p>
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
              <button type="button" onClick={() => loadDictionary()}>
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
