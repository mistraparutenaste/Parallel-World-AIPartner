import type { PersonaProfileDto } from '@parallel-world/contracts';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PersonalityPanel } from './PersonalityPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('../../shared/ipc/event-bus', () => ({
  subscribeEvent: vi.fn(() => vi.fn()),
}));

const PROFILE: PersonaProfileDto = {
  character_id: 'alice',
  name: 'アリス',
  first_person_pronoun: '私',
  user_name: 'ユーザー',
  user_address: 'あなた',
  relationship: '友人',
  speaking_style: '丁寧',
  interests: [],
  dislikes: [],
  values: [],
  background: '',
  boundaries: [],
  free_text: '',
  preset: null,
  initiative: 50,
  closeness: 50,
  humor: 50,
  response_length: 50,
  emotional_expression: 50,
  reaction_interval: 50,
  machiavellianism: 50,
  narcissism: 50,
  psychopathy: 50,
  allow_intense_dark_expression: false,
  dark_expression_acknowledgement_version: null,
};

describe('PersonalityPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') {
        return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      }
      if (command === 'get_persona_profile') return Promise.resolve(PROFILE);
      if (command === 'set_persona_profile') return Promise.resolve(PROFILE);
      return Promise.resolve(null);
    });
  });

  it('loads all personality values for the active character and shows the safety copy', async () => {
    render(<PersonalityPanel />);

    expect(await screen.findByText('アリスの性格')).toBeInTheDocument();
    expect(screen.getAllByRole('slider')).toHaveLength(9);
    expect(screen.getByLabelText('積極性')).toHaveValue('50');
    expect(screen.getByLabelText('マキャベリズム')).toHaveValue('50');
    expect(screen.getAllByText(/0 ·/)).toHaveLength(9);
    expect(screen.getAllByText(/· 100/)).toHaveLength(9);
    expect(screen.getByText(/あなたを傷つけたり、トラウマを想起させたり/)).toBeInTheDocument();
    expect(screen.getByText(/心理診断、治療または医療上の助言を目的とするものではありません/)).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith('get_persona_profile', { characterId: 'alice' });
  });

  it('saves edited values and explains when they take effect', async () => {
    render(<PersonalityPanel />);
    const initiative = await screen.findByLabelText('積極性');
    fireEvent.change(initiative, { target: { value: '72' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
      profile: { ...PROFILE, initiative: 72 },
    }));
    expect(await screen.findByText(/次のメッセージから適用されます/)).toBeInTheDocument();
  });

  it('requires explicit acknowledgement before enabling intense dark expression', async () => {
    render(<PersonalityPanel />);
    const toggle = await screen.findByRole('checkbox', { name: '強いダーク表現を許可' });
    fireEvent.click(toggle);

    const dialog = screen.getByRole('dialog', { name: '強いダーク表現の確認' });
    expect(dialog).toHaveTextContent('LLM提供元およびParallel Worldの基本的な安全保護は解除されません');
    const confirm = screen.getByRole('button', { name: '自己責任で有効にする' });
    expect(confirm).toBeDisabled();

    fireEvent.click(screen.getByRole('checkbox', { name: 'リスクを理解し、自己責任で有効にする' }));
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);
    expect(toggle).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
      profile: {
        ...PROFILE,
        allow_intense_dark_expression: true,
        dark_expression_acknowledgement_version: 1,
      },
    }));
  });

  it('turns intense expression off without confirmation and clears acknowledgement', async () => {
    const enabled = {
      ...PROFILE,
      allow_intense_dark_expression: true,
      dark_expression_acknowledgement_version: 1,
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      if (command === 'get_persona_profile') return Promise.resolve(enabled);
      return Promise.resolve(enabled);
    });
    render(<PersonalityPanel />);

    const toggle = await screen.findByRole('checkbox', { name: '強いダーク表現を許可' });
    fireEvent.click(toggle);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
      profile: {
        ...enabled,
        allow_intense_dark_expression: false,
        dark_expression_acknowledgement_version: null,
      },
    }));
  });

  it('resets all sliders and the safety opt-in before saving', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      if (command === 'get_persona_profile') {
        return Promise.resolve({
          ...PROFILE,
          initiative: 100,
          machiavellianism: 90,
          allow_intense_dark_expression: true,
          dark_expression_acknowledgement_version: 1,
        });
      }
      return Promise.resolve(PROFILE);
    });
    render(<PersonalityPanel />);
    await screen.findByText('アリスの性格');

    fireEvent.click(screen.getByRole('button', { name: '基準値に戻す' }));
    for (const slider of screen.getAllByRole('slider')) expect(slider).toHaveValue('50');
    expect(screen.getByRole('checkbox', { name: '強いダーク表現を許可' })).not.toBeChecked();
  });

  it('keeps the edited draft when saving fails', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      if (command === 'get_persona_profile') return Promise.resolve(PROFILE);
      if (command === 'set_persona_profile') return Promise.reject(new Error('disk full'));
      return Promise.resolve(null);
    });
    render(<PersonalityPanel />);
    const initiative = await screen.findByLabelText('積極性');
    fireEvent.change(initiative, { target: { value: '72' } });
    fireEvent.click(screen.getByRole('button', { name: '保存' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('性格設定を保存できません');
    expect(initiative).toHaveValue('72');
  });
});
