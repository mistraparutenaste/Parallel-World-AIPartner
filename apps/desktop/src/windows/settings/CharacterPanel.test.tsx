import type {
  CharacterManifestDto,
  CharacterSettingsDto,
} from '@parallel-world/contracts';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CharacterPanel,
  createCharacterPanelRequestGate,
} from './CharacterPanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));

const MANIFEST: CharacterManifestDto = {
  schema_version: 2,
  id: 'epsilon',
  display_name: 'Epsilon',
  renderer: {
    kind: 'live2d',
    model_path: 'C:/data/characters/epsilon/Epsilon.model3.json',
    default_expression: 'Normal',
    expressions: ['Normal', 'Smile'],
    motion_groups: [
      { name: 'Idle', motion_count: 1 },
      { name: 'Tap', motion_count: 4 },
    ],
  },
};

const STATIC_MANIFEST: CharacterManifestDto = {
  schema_version: 2,
  id: 'epsilon-static',
  display_name: 'Epsilon Static',
  renderer: {
    kind: 'static_image',
    default_expression: 'neutral',
    expressions: [
      { name: 'neutral', image_path: 'C:/data/characters/epsilon/neutral.png' },
      { name: 'happy', image_path: 'C:/data/characters/epsilon/happy.png' },
    ],
    width: 1024,
    height: 1024,
  },
};

const SETTINGS: CharacterSettingsDto = {
  schema_version: 1,
  active_character_id: 'epsilon',
  expression_idle_timeout_seconds: 20,
};

const TIMEOUT_OPTIONS = [
  ['10秒', '10'],
  ['20秒', '20'],
  ['30秒', '30'],
  ['1分', '60'],
  ['2分', '120'],
  ['5分', '300'],
  ['10分', '600'],
] as const;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function mockLoadedPanel(
  manifest: CharacterManifestDto = MANIFEST,
  settings: CharacterSettingsDto = SETTINGS,
): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'get_character_manifest') return Promise.resolve(manifest);
    if (command === 'get_character_settings') return Promise.resolve(settings);
    return Promise.resolve(undefined);
  });
}

describe('CharacterPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('lists Live2D expressions and motion groups from the renderer manifest', async () => {
    mockLoadedPanel();
    render(<CharacterPanel />);

    expect(await screen.findByRole('option', { name: 'Smile' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Tap を再生' })).toBeInTheDocument();
  });

  it('lists static expressions without rendering motion controls', async () => {
    mockLoadedPanel(STATIC_MANIFEST);
    render(<CharacterPanel />);

    expect(await screen.findByRole('option', { name: 'happy' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'モーション' })).not.toBeInTheDocument();
  });

  it('sends the selected expression through the command', async () => {
    mockLoadedPanel();
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情');

    fireEvent.change(select, { target: { value: 'Smile' } });

    expect(invokeMock).toHaveBeenCalledWith('set_expression', { name: 'Smile' });
  });

  it('starts a motion group through the command', async () => {
    mockLoadedPanel();
    render(<CharacterPanel />);
    const button = await screen.findByRole('button', { name: 'Idle を再生' });

    fireEvent.click(button);

    expect(invokeMock).toHaveBeenCalledWith('start_motion', { group: 'Idle' });
  });

  it('loads every supported idle timeout option', async () => {
    mockLoadedPanel();
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情をデフォルトに戻す時間');

    expect(select).toHaveValue('20');
    expect(screen.getByRole('option', { name: '戻さない' })).toHaveValue('never');
    for (const [label, value] of TIMEOUT_OPTIONS) {
      expect(screen.getByRole('option', { name: label })).toHaveValue(value);
    }
  });

  it('saves never as null and uses the returned settings', async () => {
    mockLoadedPanel();
    const saved = { ...SETTINGS, expression_idle_timeout_seconds: null };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'set_expression_idle_timeout') return Promise.resolve(saved);
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情をデフォルトに戻す時間');

    fireEvent.change(select, { target: { value: 'never' } });

    await waitFor(() => expect(select).toHaveValue('never'));
    expect(invokeMock).toHaveBeenCalledWith('set_expression_idle_timeout', {
      timeoutSeconds: null,
    });
  });

  it('saves an integer timeout and applies the returned settings without losing the active id', async () => {
    const saved: CharacterSettingsDto = {
      ...SETTINGS,
      active_character_id: 'epsilon',
      expression_idle_timeout_seconds: 30,
    };
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'set_expression_idle_timeout') return Promise.resolve(saved);
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情をデフォルトに戻す時間');

    fireEvent.change(select, { target: { value: '30' } });

    await waitFor(() => expect(select).toHaveValue('30'));
    expect(saved.active_character_id).toBe(SETTINGS.active_character_id);
    expect(invokeMock).toHaveBeenCalledWith('set_expression_idle_timeout', {
      timeoutSeconds: 30,
    });
  });

  it('does not let an older save result overwrite the newest settings', async () => {
    const older = deferred<CharacterSettingsDto>();
    const latest = deferred<CharacterSettingsDto>();
    let saveCount = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'set_expression_idle_timeout') {
        saveCount += 1;
        return saveCount === 1 ? older.promise : latest.promise;
      }
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情をデフォルトに戻す時間');

    fireEvent.change(select, { target: { value: '60' } });
    fireEvent.change(select, { target: { value: '30' } });
    expect(saveCount).toBe(2);

    latest.resolve({ ...SETTINGS, expression_idle_timeout_seconds: 30 });
    await waitFor(() => expect(select).toHaveValue('30'));

    await act(async () => {
      older.resolve({ ...SETTINGS, expression_idle_timeout_seconds: 60 });
      await older.promise;
    });
    expect(select).toHaveValue('30');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('retains the previous timeout and shows the existing alert path on save failure', async () => {
    mockLoadedPanel();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'set_expression_idle_timeout') return Promise.reject(new Error('disk full'));
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);
    const select = await screen.findByLabelText('表情をデフォルトに戻す時間');

    fireEvent.change(select, { target: { value: '60' } });

    expect(await screen.findByRole('alert')).toHaveTextContent('表情の復帰時間を保存できません');
    expect(select).toHaveValue('20');
  });

  it('shows an alert when the manifest cannot be loaded', async () => {
    invokeMock.mockImplementation((command: string) => command === 'get_character_manifest'
      ? Promise.reject(new Error('no character model'))
      : Promise.resolve(SETTINGS));
    render(<CharacterPanel />);

    expect(await screen.findByRole('alert')).toHaveTextContent('キャラクターモデルを読み込めません');
  });

  it('reloads the panel when retry is clicked after a failure', async () => {
    let manifestAttempts = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'get_character_manifest') {
        manifestAttempts += 1;
        return manifestAttempts === 1
          ? Promise.reject(new Error('no character model'))
          : Promise.resolve(MANIFEST);
      }
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);
    const retry = await screen.findByRole('button', { name: '再読み込み' });

    fireEvent.click(retry);

    expect(await screen.findByRole('option', { name: 'Smile' })).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});

describe('character panel request generation guard', () => {
  it('invalidates every pending request on unmount and across a remount', () => {
    const gate = createCharacterPanelRequestGate();
    gate.mount();
    const beforeUnmount = gate.begin();

    gate.unmount();
    expect(gate.isCurrent(beforeUnmount)).toBe(false);

    gate.mount();
    expect(gate.isCurrent(beforeUnmount)).toBe(false);
  });

  it('accepts only the latest request generation', () => {
    const gate = createCharacterPanelRequestGate();
    gate.mount();
    const older = gate.begin();
    const latest = gate.begin();

    expect(gate.isCurrent(older)).toBe(false);
    expect(gate.isCurrent(latest)).toBe(true);
  });
});
