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
  base_url: 'http://127.0.0.1:8080/v1',
  model: 'default',
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
});
