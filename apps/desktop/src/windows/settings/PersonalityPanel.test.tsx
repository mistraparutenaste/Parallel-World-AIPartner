import type {
  DarkExpressionSafetySettingsDto,
  PersonaProfileDto,
} from '@parallel-world/contracts';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
  interests: ['紅茶'],
  dislikes: [],
  values: ['誠実'],
  background: '',
  boundaries: [],
  free_text: '',
  example_utterances: [],
  initiative: 50,
  closeness: 50,
  humor: 50,
  response_length: 50,
  emotional_expression: 50,
  reaction_interval: 50,
  machiavellianism: 50,
  narcissism: 50,
  psychopathy: 50,
  sadism: 50,
  allow_intense_dark_expression: false,
  dark_expression_acknowledgement_version: null,
};

const SAFETY: DarkExpressionSafetySettingsDto = {
  schema_version: 1,
  safe_word: null,
  dark_expression_paused: false,
};

describe('PersonalityPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'get_character_manifest') {
        return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      }
      if (command === 'get_persona_profile') return Promise.resolve(PROFILE);
      if (command === 'get_dark_expression_safety_settings') return Promise.resolve(SAFETY);
      if (command === 'set_persona_profile') return Promise.resolve(args?.profile);
      if (command === 'set_safe_word') {
        return Promise.resolve({
          ...SAFETY,
          safe_word: args?.safeWord || null,
        });
      }
      if (command === 'resume_dark_expression') {
        return Promise.resolve({ ...SAFETY, dark_expression_paused: false });
      }
      return Promise.resolve(null);
    });
  });

  it('starts with narrative context, then six conversation traits and four dark traits', async () => {
    render(<PersonalityPanel />);

    expect(await screen.findByText('アリスの性格')).toBeInTheDocument();
    const headings = screen.getAllByRole('group').map((group) => group.getAttribute('aria-label'));
    expect(headings).toEqual(expect.arrayContaining(['この子について', '会話の傾向']));
    expect(screen.getByText('高度な設定').closest('details')).not.toHaveAttribute('open');
    expect(screen.getByLabelText('名前')).toHaveValue('アリス');
    expect(screen.getByLabelText('興味')).toHaveValue('紅茶');
    const identity = screen.getByRole('region', { name: '基本情報' });
    const voice = screen.getByRole('region', { name: '話し方と嗜好' });
    const story = screen.getByRole('region', { name: '背景と境界' });
    expect(within(identity).getByLabelText('関係性')).toBeInTheDocument();
    expect(within(voice).getByLabelText('話し方')).toBeInTheDocument();
    expect(within(voice).getByLabelText('価値観')).toBeInTheDocument();
    expect(within(story).getByLabelText('背景').tagName).toBe('TEXTAREA');
    expect(within(story).getByLabelText('境界').tagName).toBe('TEXTAREA');
    expect(within(story).getByLabelText('自由記述').tagName).toBe('TEXTAREA');
    expect(screen.getAllByRole('slider')).toHaveLength(10);
    expect(screen.getByLabelText('積極性')).toHaveValue('50');
    fireEvent.click(screen.getByText('高度な設定'));
    expect(screen.getByText('高度な設定').closest('details')).toHaveAttribute('open');
    expect(screen.getByLabelText('サディズム')).toHaveValue('50');
    expect(screen.getAllByText('0 · 低い')).toHaveLength(4);
    expect(screen.getAllByText('高い · 100')).toHaveLength(4);
    expect(screen.getByText(/あなたを傷つけたり、トラウマを想起させたり/)).toBeInTheDocument();
    expect(screen.getByText(/心理診断、治療または医療上の助言を目的とするものではありません/)).toBeInTheDocument();
  });

  it('saves sliders immediately and rolls the visible value back on failure', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') {
        return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      }
      if (command === 'get_persona_profile') return Promise.resolve(PROFILE);
      if (command === 'get_dark_expression_safety_settings') return Promise.resolve(SAFETY);
      if (command === 'set_persona_profile') return Promise.reject(new Error('disk full'));
      return Promise.resolve(null);
    });
    render(<PersonalityPanel />);
    const initiative = await screen.findByLabelText('積極性');

    fireEvent.change(initiative, { target: { value: '72' } });

    expect(await screen.findByRole('alert')).toHaveTextContent('性格設定を変更できません');
    expect(initiative).toHaveValue('50');
  });

  it('shows local apply and revert actions only while narrative text is dirty', async () => {
    render(<PersonalityPanel />);
    const name = await screen.findByLabelText('名前');
    expect(screen.queryByRole('button', { name: 'この子についてを適用' })).not.toBeInTheDocument();

    fireEvent.change(name, { target: { value: 'Alice Nova' } });

    expect(screen.getByRole('button', { name: 'この子についてを適用' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '元に戻す' }));
    expect(name).toHaveValue('アリス');

    fireEvent.change(name, { target: { value: 'Alice Nova' } });
    fireEvent.click(screen.getByRole('button', { name: 'この子についてを適用' }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
        profile: { ...PROFILE, name: 'Alice Nova' },
      }),
    );
  });

  it('requires acknowledgement version 2 before enabling intense dark expression', async () => {
    render(<PersonalityPanel />);
    fireEvent.click(await screen.findByText('高度な設定'));
    const toggle = await screen.findByRole('switch', { name: '強いダーク表現を許可' });
    fireEvent.click(toggle);

    const dialog = screen.getByRole('dialog', { name: '強いダーク表現の確認' });
    expect(dialog).toHaveTextContent('セーフワードが設定されていません');
    const confirm = within(dialog).getByRole('button', { name: '自己責任で有効にする' });
    expect(confirm).toBeDisabled();

    fireEvent.click(within(dialog).getByRole('checkbox', {
      name: 'リスクを理解し、自己責任で有効にする',
    }));
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
        profile: {
          ...PROFILE,
          allow_intense_dark_expression: true,
          dark_expression_acknowledgement_version: 2,
        },
      }),
    );
    expect(toggle).toBeChecked();
  });

  it('configures the user-wide safeword and resumes a persisted pause explicitly', async () => {
    invokeMock.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === 'get_character_manifest') {
        return Promise.resolve({ id: 'alice', display_name: 'アリス' });
      }
      if (command === 'get_persona_profile') return Promise.resolve(PROFILE);
      if (command === 'get_dark_expression_safety_settings') {
        return Promise.resolve({ ...SAFETY, dark_expression_paused: true });
      }
      if (command === 'set_safe_word') {
        return Promise.resolve({
          ...SAFETY,
          safe_word: args?.safeWord,
          dark_expression_paused: true,
        });
      }
      if (command === 'resume_dark_expression') return Promise.resolve(SAFETY);
      return Promise.resolve(args?.profile);
    });
    render(<PersonalityPanel />);
    fireEvent.click(await screen.findByText('高度な設定'));
    const input = await screen.findByLabelText('セーフワード（推奨）');
    expect(screen.getByText('ダーク表現を停止しています。')).toBeInTheDocument();

    fireEvent.change(input, { target: { value: '停止' } });
    fireEvent.click(screen.getByRole('button', { name: 'セーフワードを適用' }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('set_safe_word', { safeWord: '停止' }),
    );
    expect(screen.queryByText(/セーフワードが設定されていません/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'ダーク表現を再開' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('resume_dark_expression'));
  });

  it('applies a preset to all six general traits', async () => {
    render(<PersonalityPanel />);
    fireEvent.click(await screen.findByRole('button', { name: '元気' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_persona_profile', {
      profile: {
        ...PROFILE,
        initiative: 80,
        closeness: 75,
        humor: 80,
        response_length: 55,
        emotional_expression: 85,
        reaction_interval: 80,
      },
    }));
  });
});
