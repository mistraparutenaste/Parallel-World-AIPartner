import type { LlmSettingsDto } from '@parallel-world/contracts';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LlmPanel } from './LlmPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const SETTINGS: LlmSettingsDto = {
  schema_version: 1,
  provider: 'local',
  base_url: 'http://127.0.0.1:8080/v1',
  model: 'default',
  api_key: '',
  api_key_configured: false,
  clear_api_key: false,
  allow_remote: false,
  system_prompt: '規則',
  character_prompt: 'キャラ',
  strip_emoji: true,
};

describe('LlmPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_llm_settings') {
        return Promise.resolve(SETTINGS);
      }
      return Promise.resolve(null);
    });
  });

  it('loads and shows the persisted settings', async () => {
    render(<LlmPanel />);
    expect(await screen.findByLabelText('接続先 (OpenAI互換)')).toHaveValue(
      'http://127.0.0.1:8080/v1',
    );
    expect(screen.getByLabelText('モデル名')).toHaveValue('default');
  });

  it('saves edited settings through set_llm_settings', async () => {
    render(<LlmPanel />);
    const model = await screen.findByLabelText('モデル名');

    fireEvent.change(model, { target: { value: 'qwen2.5-7b' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_llm_settings', {
      settings: { ...SETTINGS, model: 'qwen2.5-7b' },
    });
  });

  it('applies the Gemini preset and submits a replacement API key', async () => {
    render(<LlmPanel />);
    const provider = await screen.findByLabelText('プロバイダー');

    fireEvent.change(provider, { target: { value: 'gemini' } });
    fireEvent.change(screen.getByLabelText('APIキー'), {
      target: { value: 'gemini-secret' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(invokeMock).toHaveBeenCalledWith('set_llm_settings', {
      settings: {
        ...SETTINGS,
        provider: 'gemini',
        base_url: 'https://generativelanguage.googleapis.com/v1beta/openai',
        allow_remote: true,
        api_key: 'gemini-secret',
      },
    });
  });

  it('shows the OpenCode Zen Chat Completions limitation', async () => {
    render(<LlmPanel />);
    fireEvent.change(await screen.findByLabelText('プロバイダー'), {
      target: { value: 'opencode_zen' },
    });

    expect(screen.getByText(/Chat Completions対応モデル/)).toBeInTheDocument();
    expect(screen.getByLabelText('接続先 (OpenAI互換)')).toHaveValue(
      'https://opencode.ai/zen/v1',
    );
  });
});
