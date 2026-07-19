import type {
  TtsSettingsDto,
  TtsVoiceDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TtsPanel } from './TtsPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const AIVIS_SETTINGS: TtsSettingsDto = {
  schema_version: 1,
  enabled: true,
  base_url: 'http://127.0.0.1:10101',
  engine: 'aivis',
  voice_id: '888753760',
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

describe('TtsPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
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
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_tts_settings', {
      settings: {
        ...AIVIS_SETTINGS,
        engine: 'irodori',
        base_url: 'http://127.0.0.1:8088',
        voice_id: 'irodori-voice',
      },
    });
    expect(invokeMock).toHaveBeenCalledWith('list_tts_voices');
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
    expect(screen.getByText(/同意を得た参照音声のみ/)).toBeVisible();
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
      surface: 'STT',
      pronunciation: 'エスティーティー',
      accentType: 0,
    });

    fireEvent.click(screen.getByRole('button', { name: 'LLMを削除' }));
    expect(invokeMock).toHaveBeenCalledWith('delete_user_dict_word', {
      uuid: 'aaaa-bbbb',
    });
  });
});
