import type {
  TtsSettingsDto,
  TtsVoiceDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TtsPanel } from './TtsPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const openMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: openMock,
}));

const AIVIS_SETTINGS: TtsSettingsDto = {
  schema_version: 1,
  enabled: true,
  base_url: 'http://127.0.0.1:10101',
  engine: 'aivis',
  voice_id: '888753760',
  irodori_lora_adapter: '',
  style_id: 888753760,
  volume: 1,
  speed: 1,
};

const VOICES: TtsVoiceDto[] = [
  { id: '888753760', label: 'Anneli / ノーマル' },
  { id: 'irodori-voice', label: 'Irodori Voice' },
];

const WORDS: UserDictWordDto[] = [
  {
    uuid: 'aaaa-bbbb',
    surface: 'LLM',
    pronunciation: 'エルエルエム',
    accent_type: 1,
  },
];

function mockSettings(settings: TtsSettingsDto = AIVIS_SETTINGS) {
  invokeMock.mockImplementation((command: string) => {
    switch (command) {
      case 'get_tts_settings':
        return Promise.resolve(settings);
      case 'get_irodori_install_state':
        return Promise.resolve({
          schema_version: 1,
          installed: true,
          install_root: 'C:\\irodori',
          missing_artifacts: [],
        });
      case 'list_tts_voices':
        return Promise.resolve(VOICES);
      case 'list_user_dict':
        return Promise.resolve(WORDS);
      case 'add_user_dict_word':
        return Promise.resolve('new-uuid');
      default:
        return Promise.resolve(null);
    }
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe('TtsPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openMock.mockReset();
    openMock.mockResolvedValue(null);
    mockSettings();
  });

  it('loads persisted Aivis settings and shows its dictionary', async () => {
    render(<TtsPanel />);

    expect(await screen.findByLabelText('TTSエンジン')).toHaveValue('aivis');
    expect(screen.getByLabelText('接続先 (AivisSpeech Engine)')).toHaveValue(
      'http://127.0.0.1:10101',
    );
    expect(screen.getByRole('heading', { name: 'ユーザー辞書' })).toBeVisible();
  });

  it('switches an engine default URL and resets the selected voice', async () => {
    render(<TtsPanel />);

    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'irodori' },
    });

    expect(screen.getByLabelText('接続先 (irodori-TTS)')).toHaveValue(
      'http://127.0.0.1:8088',
    );
    expect(screen.getByLabelText('音声')).toHaveValue('');
    expect(screen.queryByRole('heading', { name: 'ユーザー辞書' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '音声一覧を取得' }));
    expect(invokeMock).toHaveBeenCalledWith('list_tts_voices', {
      engine: 'irodori',
      baseUrl: 'http://127.0.0.1:8088',
    });
  });

  it('switches the irodori default URL back to the Aivis default', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    render(<TtsPanel />);

    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'aivis' },
    });

    expect(screen.getByLabelText('接続先 (AivisSpeech Engine)')).toHaveValue(
      'http://127.0.0.1:10101',
    );
    expect(screen.getByLabelText('音声')).toHaveValue('');
    fireEvent.click(screen.getByRole('button', { name: '音声一覧を取得' }));
    expect(invokeMock).toHaveBeenCalledWith('list_tts_voices', {
      engine: 'aivis',
      baseUrl: 'http://127.0.0.1:10101',
    });
  });

  it('preserves a custom URL when switching engines', async () => {
    mockSettings({ ...AIVIS_SETTINGS, base_url: 'http://127.0.0.1:18080' });
    render(<TtsPanel />);

    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'irodori' },
    });

    expect(screen.getByLabelText('接続先 (irodori-TTS)')).toHaveValue(
      'http://127.0.0.1:18080',
    );
    fireEvent.click(screen.getByRole('button', { name: '音声一覧を取得' }));
    expect(invokeMock).toHaveBeenCalledWith('list_tts_voices', {
      engine: 'irodori',
      baseUrl: 'http://127.0.0.1:18080',
    });
  });

  it('fetches normalized voices and saves the selected irodori string id', async () => {
    render(<TtsPanel />);
    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'irodori' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: '音声一覧を取得' }),
    );

    expect(
      await screen.findByRole('option', { name: 'Irodori Voice' }),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('音声'), {
      target: { value: 'irodori-voice' },
    });
    fireEvent.change(screen.getByLabelText('LoRA adapter path'), {
      target: { value: 'C:/models/adapters/character-a' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_tts_settings', {
      settings: {
        ...AIVIS_SETTINGS,
        engine: 'irodori',
        base_url: 'http://127.0.0.1:8088',
        voice_id: 'irodori-voice',
        irodori_lora_adapter: 'C:/models/adapters/character-a',
      },
    });
    expect(invokeMock).toHaveBeenCalledWith('list_tts_voices', {
      engine: 'irodori',
      baseUrl: 'http://127.0.0.1:8088',
    });
  });

  it('fills the LoRA adapter path from the folder picker', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    openMock.mockResolvedValue('/Users/me/models/adapters/character-a');
    render(<TtsPanel />);

    fireEvent.click(
      await screen.findByRole('button', { name: 'フォルダを選択' }),
    );

    await waitFor(() => {
      expect(screen.getByLabelText('LoRA adapter path')).toHaveValue(
        '/Users/me/models/adapters/character-a',
      );
    });
    expect(openMock).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
    });
  });

  it('resolves a picked adapter file to its directory', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    openMock.mockResolvedValue(
      '/Users/me/models/adapters/character-a/adapter_model.safetensors',
    );
    render(<TtsPanel />);

    fireEvent.click(
      await screen.findByRole('button', { name: 'ファイルから選択' }),
    );

    await waitFor(() => {
      expect(screen.getByLabelText('LoRA adapter path')).toHaveValue(
        '/Users/me/models/adapters/character-a',
      );
    });
    expect(openMock).toHaveBeenCalledWith({
      multiple: false,
      filters: [
        {
          name: 'LoRA adapter',
          extensions: ['safetensors', 'bin', 'pt', 'json'],
        },
        { name: 'すべてのファイル', extensions: ['*'] },
      ],
    });
  });

  it('resolves a picked adapter file below a Windows drive root', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    openMock.mockResolvedValue('C:\\adapter_model.safetensors');
    render(<TtsPanel />);

    fireEvent.click(
      await screen.findByRole('button', { name: 'ファイルから選択' }),
    );

    await waitFor(() => {
      expect(screen.getByLabelText('LoRA adapter path')).toHaveValue('C:\\');
    });
  });

  it('keeps the LoRA adapter path when the folder picker is cancelled', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
      irodori_lora_adapter: '/Users/me/models/adapters/character-a',
    });
    openMock.mockResolvedValue(null);
    render(<TtsPanel />);

    fireEvent.click(
      await screen.findByRole('button', { name: 'フォルダを選択' }),
    );

    await waitFor(() => {
      expect(openMock).toHaveBeenCalledWith({
        directory: true,
        multiple: false,
        defaultPath: '/Users/me/models/adapters/character-a',
      });
    });
    expect(screen.getByLabelText('LoRA adapter path')).toHaveValue(
      '/Users/me/models/adapters/character-a',
    );
  });

  it('ignores an old Aivis voice response after switching to irodori', async () => {
    const aivisVoices = deferred<TtsVoiceDto[]>();
    invokeMock.mockImplementation((command: string, args?: unknown) => {
      if (command === 'get_tts_settings') {
        return Promise.resolve(AIVIS_SETTINGS);
      }
      if (command === 'list_tts_voices') {
        const target = args as { engine: string };
        return target.engine === 'aivis'
          ? aivisVoices.promise
          : Promise.resolve([{ id: 'irodori-voice', label: 'Irodori Voice' }]);
      }
      return Promise.resolve(null);
    });
    render(<TtsPanel />);

    fireEvent.click(
      await screen.findByRole('button', { name: '音声一覧を取得' }),
    );
    fireEvent.change(screen.getByLabelText('TTSエンジン'), {
      target: { value: 'irodori' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: '音声一覧を取得' }),
    );
    expect(
      await screen.findByRole('option', { name: 'Irodori Voice' }),
    ).toBeInTheDocument();

    await act(async () => {
      aivisVoices.resolve([{ id: 'old-aivis', label: 'Old Aivis Voice' }]);
      await aivisVoices.promise;
    });
    await waitFor(() => {
      expect(
        screen.queryByRole('option', { name: 'Old Aivis Voice' }),
      ).not.toBeInTheDocument();
    });
  });

  it('shows the backend error when saving irodori without a voice', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_tts_settings') {
        return Promise.resolve(AIVIS_SETTINGS);
      }
      if (command === 'set_tts_settings') {
        return Promise.reject(new Error('voice_id must not be empty'));
      }
      return Promise.resolve(null);
    });
    render(<TtsPanel />);

    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'irodori' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(await screen.findByRole('status')).toHaveTextContent(
      'voice_id must not be empty',
    );
  });

  it('shows the irodori operational and consent notices', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    render(<TtsPanel />);

    await screen.findByLabelText('接続先 (irodori-TTS)');
    expect(screen.getByText(/GPUを推奨/)).toBeVisible();
    expect(screen.getByText(/初回の音声合成.*モデル.*時間/)).toBeVisible();
    expect(screen.getByText(/IRODORI_COMPILE_MODEL=false/)).toBeVisible();
    expect(screen.getByText(/同意を得た参照音声のみ/)).toBeVisible();
  });

  it('offers the managed installer when irodori artifacts are missing', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_tts_settings') {
        return Promise.resolve({
          ...AIVIS_SETTINGS,
          engine: 'irodori',
          base_url: 'http://127.0.0.1:8088',
          voice_id: 'irodori-voice',
        });
      }
      if (command === 'get_irodori_install_state') {
        return Promise.resolve({
          schema_version: 1,
          installed: false,
          install_root: 'C:\\irodori',
          missing_artifacts: ['models/model.safetensors'],
        });
      }
      return Promise.resolve(null);
    });
    render(<TtsPanel />);
    fireEvent.click(await screen.findByRole('button', { name: 'インストールを開始' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('install_irodori'));
    expect(await screen.findByRole('status')).toHaveTextContent('別ウィンドウで起動');
  });

  it('uses an engine-specific voice error message', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_tts_settings') {
        return Promise.resolve({
          ...AIVIS_SETTINGS,
          engine: 'irodori',
          base_url: 'http://127.0.0.1:8088',
        });
      }
      if (command === 'list_tts_voices') {
        return Promise.reject(new Error('offline'));
      }
      return Promise.resolve(null);
    });
    render(<TtsPanel />);
    fireEvent.click(
      await screen.findByRole('button', { name: '音声一覧を取得' }),
    );

    expect(await screen.findByRole('status')).toHaveTextContent(
      'irodori-TTS の音声一覧を取得できません',
    );
  });

  it('lists, adds and deletes Aivis user dictionary words', async () => {
    render(<TtsPanel />);
    fireEvent.click(
      await screen.findByRole('button', { name: '辞書を読み込む' }),
    );
    expect(await screen.findByText(/LLM（エルエルエム）/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('単語'), {
      target: { value: 'STT' },
    });
    fireEvent.change(screen.getByLabelText('読み（カタカナ）'), {
      target: { value: 'エスティーティー' },
    });
    fireEvent.click(screen.getByRole('button', { name: '単語を追加' }));
    expect(invokeMock).toHaveBeenCalledWith('add_user_dict_word', {
      engine: 'aivis',
      baseUrl: 'http://127.0.0.1:10101',
      surface: 'STT',
      pronunciation: 'エスティーティー',
      accentType: 0,
    });

    fireEvent.click(screen.getByRole('button', { name: 'LLMを削除' }));
    expect(invokeMock).toHaveBeenCalledWith('delete_user_dict_word', {
      engine: 'aivis',
      baseUrl: 'http://127.0.0.1:10101',
      uuid: 'aaaa-bbbb',
    });
  });

  it('uses unsaved Aivis connection values for dictionary commands', async () => {
    mockSettings({
      ...AIVIS_SETTINGS,
      engine: 'irodori',
      base_url: 'http://127.0.0.1:8088',
      voice_id: 'irodori-voice',
    });
    render(<TtsPanel />);

    fireEvent.change(await screen.findByLabelText('TTSエンジン'), {
      target: { value: 'aivis' },
    });
    fireEvent.click(
      screen.getByRole('button', { name: '辞書を読み込む' }),
    );

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('list_user_dict', {
        engine: 'aivis',
        baseUrl: 'http://127.0.0.1:10101',
      });
    });

    fireEvent.change(screen.getByLabelText('単語'), {
      target: { value: 'API' },
    });
    fireEvent.change(screen.getByLabelText('読み（カタカナ）'), {
      target: { value: 'エーピーアイ' },
    });
    fireEvent.click(screen.getByRole('button', { name: '単語を追加' }));
    expect(invokeMock).toHaveBeenCalledWith('add_user_dict_word', {
      engine: 'aivis',
      baseUrl: 'http://127.0.0.1:10101',
      surface: 'API',
      pronunciation: 'エーピーアイ',
      accentType: 0,
    });

    fireEvent.click(screen.getByRole('button', { name: 'LLMを削除' }));
    expect(invokeMock).toHaveBeenCalledWith('delete_user_dict_word', {
      engine: 'aivis',
      baseUrl: 'http://127.0.0.1:10101',
      uuid: 'aaaa-bbbb',
    });
  });
});
