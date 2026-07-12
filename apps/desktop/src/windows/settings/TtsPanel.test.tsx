import type {
  TtsSettingsDto,
  TtsSpeakerDto,
  UserDictWordDto,
} from '@parallel-world/contracts';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TtsPanel } from './TtsPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const SETTINGS: TtsSettingsDto = {
  schema_version: 1,
  enabled: true,
  base_url: 'http://127.0.0.1:10101',
  style_id: 888753760,
  volume: 1,
  speed: 1,
};

const SPEAKERS: TtsSpeakerDto[] = [
  { name: 'Anneli', style_name: 'ノーマル', style_id: 888753760 },
  { name: 'White', style_name: 'ノーマル', style_id: 706073888 },
];

const WORDS: UserDictWordDto[] = [
  {
    uuid: 'aaaa-bbbb',
    surface: 'ＬＬＭ',
    pronunciation: 'エルエルエム',
    accent_type: 1,
  },
];

describe('TtsPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      switch (command) {
        case 'get_tts_settings':
          return Promise.resolve(SETTINGS);
        case 'list_tts_speakers':
          return Promise.resolve(SPEAKERS);
        case 'list_user_dict':
          return Promise.resolve(WORDS);
        case 'add_user_dict_word':
          return Promise.resolve('new-uuid');
        default:
          return Promise.resolve(null);
      }
    });
  });

  it('loads and shows the persisted settings', async () => {
    render(<TtsPanel />);
    expect(
      await screen.findByLabelText('接続先 (AivisSpeech Engine)'),
    ).toHaveValue('http://127.0.0.1:10101');
    expect(screen.getByLabelText('音声で読み上げる')).toBeChecked();
  });

  it('saves edited settings through set_tts_settings', async () => {
    render(<TtsPanel />);
    const enabled = await screen.findByLabelText('音声で読み上げる');

    fireEvent.click(enabled);
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_tts_settings', {
      settings: { ...SETTINGS, enabled: false },
    });
  });

  it('fetches speakers and selects a style', async () => {
    render(<TtsPanel />);
    fireEvent.click(
      await screen.findByRole('button', { name: '話者一覧を取得' }),
    );

    expect(
      await screen.findByRole('option', { name: 'White / ノーマル' }),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('話者スタイル'), {
      target: { value: '706073888' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_tts_settings', {
      settings: { ...SETTINGS, style_id: 706073888 },
    });
  });

  it('lists, adds and deletes user dictionary words', async () => {
    render(<TtsPanel />);
    fireEvent.click(
      await screen.findByRole('button', { name: '辞書を読み込む' }),
    );
    expect(await screen.findByText(/ＬＬＭ（エルエルエム）/)).toBeInTheDocument();

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

    fireEvent.click(screen.getByRole('button', { name: 'ＬＬＭを削除' }));
    expect(invokeMock).toHaveBeenCalledWith('delete_user_dict_word', {
      uuid: 'aaaa-bbbb',
    });
  });
});
