import type {
  CharacterManifestDto,
  CharacterSettingsDto,
  CharacterSetupDto,
} from '@parallel-world/contracts';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CharacterPanel,
  createCharacterPanelRequestGate,
} from './CharacterPanel';

const invokeMock = vi.hoisted(() => vi.fn());
const openMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: openMock,
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
  schema_version: 2,
  active_character_id: 'epsilon',
  live2d_character_id: 'epsilon',
  static_image_character_id: 'epsilon-static',
  expression_idle_timeout_seconds: 20,
};

const SETUP: CharacterSetupDto = {
  schema_version: 1,
  active_renderer: 'live2d',
  live2d: {
    kind: 'live2d',
    configured: true,
    display_name: 'Epsilon Live2D',
    file_name: 'Epsilon.model3.json',
    import_enabled: false,
    active: true,
  },
  static_image: {
    kind: 'static_image',
    configured: true,
    display_name: 'Epsilon Static',
    file_name: 'epsilon.webp',
    import_enabled: true,
    active: false,
  },
};

function setupWith(
  overrides: Partial<Pick<CharacterSetupDto, 'active_renderer' | 'live2d' | 'static_image'>>,
): CharacterSetupDto {
  return { ...SETUP, ...overrides };
}

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
  setup: CharacterSetupDto = SETUP,
): void {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'get_character_setup') return Promise.resolve(setup);
    if (command === 'get_character_manifest') return Promise.resolve(manifest);
    if (command === 'get_character_settings') return Promise.resolve(settings);
    return Promise.resolve(undefined);
  });
}

describe('CharacterPanel', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openMock.mockReset();
  });

  it('renders setup cards and an active two-choice renderer selector from setup state', async () => {
    mockLoadedPanel();
    render(<CharacterPanel />);

    const group = await screen.findByRole('radiogroup', { name: '表示するキャラクター形式' });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Live2D' })).toBeChecked();
    expect(screen.getByRole('radio', { name: '静止画' })).not.toBeChecked();
    expect(screen.getByRole('heading', { name: 'Live2D' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '静止画' })).toBeInTheDocument();
    expect(screen.getByText('Epsilon.model3.json')).toBeInTheDocument();
    expect(screen.getByText('epsilon.webp')).toBeInTheDocument();
  });

  it('disables an unconfigured renderer choice but keeps a configured inactive choice switchable', async () => {
    const unconfiguredStatic = setupWith({
      static_image: {
        ...SETUP.static_image,
        configured: false,
        display_name: null,
        file_name: null,
      },
    });
    mockLoadedPanel(MANIFEST, SETTINGS, unconfiguredStatic);
    const first = render(<CharacterPanel />);
    expect(await screen.findByRole('radio', { name: '静止画' })).toBeDisabled();
    first.unmount();

    const inactiveLive2d = setupWith({
      active_renderer: 'static_image',
      live2d: { ...SETUP.live2d, active: false },
      static_image: { ...SETUP.static_image, active: true },
    });
    mockLoadedPanel(STATIC_MANIFEST, { ...SETTINGS, active_character_id: 'epsilon-static' }, inactiveLive2d);
    render(<CharacterPanel />);
    expect(await screen.findByRole('radio', { name: 'Live2D' })).toBeEnabled();
  });

  it('explains release Live2D import while leaving configured Live2D switchable', async () => {
    const inactiveLive2d = setupWith({
      active_renderer: 'static_image',
      live2d: { ...SETUP.live2d, active: false, import_enabled: false },
      static_image: { ...SETUP.static_image, active: true },
    });
    mockLoadedPanel(STATIC_MANIFEST, { ...SETTINGS, active_character_id: 'epsilon-static' }, inactiveLive2d);
    render(<CharacterPanel />);

    expect(await screen.findByRole('radio', { name: 'Live2D' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Live2Dファイルを選択' })).toBeDisabled();
    expect(screen.getByText('任意のLive2D読み込みは開発ビルドでのみ利用できます。')).toBeInTheDocument();
  });

  it('treats native dialog cancellation as silent and does not import', async () => {
    mockLoadedPanel();
    openMock.mockResolvedValue(null);
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('button', { name: '静止画ファイルを選択' }));
    await waitFor(() => expect(openMock).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: '静止画', extensions: ['png', 'webp'] }],
    }));
    expect(invokeMock).not.toHaveBeenCalledWith('import_character_asset', expect.anything());
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('imports an inactive static image without switching and explains the toggle', async () => {
    const imported = setupWith({
      static_image: {
        ...SETUP.static_image,
        display_name: 'New Still',
        file_name: 'new-still.png',
      },
    });
    mockLoadedPanel();
    openMock.mockResolvedValue('C:/images/new-still.png');
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'import_character_asset') return Promise.resolve(imported);
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('button', { name: '静止画ファイルを選択' }));

    expect(await screen.findByRole('status')).toHaveTextContent('トグルで切替できます');
    expect(invokeMock).toHaveBeenCalledWith('import_character_asset', {
      kind: 'static_image',
      sourcePath: 'C:/images/new-still.png',
    });
    expect(screen.getByRole('radio', { name: 'Live2D' })).toBeChecked();
    expect(invokeMock).not.toHaveBeenCalledWith('set_active_character_renderer', expect.anything());
  });

  it('updates the active renderer card immediately after a same-kind import', async () => {
    const developmentSetup = setupWith({
      live2d: { ...SETUP.live2d, import_enabled: true },
    });
    const imported = setupWith({
      live2d: {
        ...developmentSetup.live2d,
        display_name: 'New Live2D',
        file_name: 'New.model3.json',
      },
    });
    mockLoadedPanel(MANIFEST, SETTINGS, developmentSetup);
    openMock.mockResolvedValue('C:/models/New.model3.json');
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(developmentSetup);
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'import_character_asset') return Promise.resolve(imported);
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('button', { name: 'Live2Dファイルを選択' }));

    expect(await screen.findByText('New.model3.json')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('表示中のアセットを更新しました');
  });

  it('preserves the prior setup when import fails', async () => {
    mockLoadedPanel();
    openMock.mockResolvedValue('C:/images/broken.png');
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'import_character_asset') return Promise.reject(new Error('decode failed'));
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('button', { name: '静止画ファイルを選択' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('読み込めません');
    expect(screen.getByText('epsilon.webp')).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Live2D' })).toBeChecked();
  });

  it('switches renderer, reloads the manifest once, and updates renderer-specific controls', async () => {
    const switched = setupWith({
      active_renderer: 'static_image',
      live2d: { ...SETUP.live2d, active: false },
      static_image: { ...SETUP.static_image, active: true },
    });
    let manifestLoads = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'get_character_manifest') {
        manifestLoads += 1;
        return Promise.resolve(manifestLoads === 1 ? MANIFEST : STATIC_MANIFEST);
      }
      if (command === 'set_active_character_renderer') return Promise.resolve(switched);
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('radio', { name: '静止画' }));

    expect(await screen.findByRole('option', { name: 'happy' })).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'モーション' })).not.toBeInTheDocument();
    expect(manifestLoads).toBe(2);
    expect(invokeMock).toHaveBeenCalledWith('set_active_character_renderer', { kind: 'static_image' });
  });

  it('preserves the prior selection when switching renderer fails', async () => {
    mockLoadedPanel();
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
      if (command === 'get_character_manifest') return Promise.resolve(MANIFEST);
      if (command === 'get_character_settings') return Promise.resolve(SETTINGS);
      if (command === 'set_active_character_renderer') return Promise.reject(new Error('save failed'));
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    fireEvent.click(await screen.findByRole('radio', { name: '静止画' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('切り替えできません');
    expect(screen.getByRole('radio', { name: 'Live2D' })).toBeChecked();
  });

  it('keeps setup controls visible when no manifest is selected', async () => {
    const emptySetup = setupWith({
      active_renderer: null,
      live2d: { ...SETUP.live2d, configured: false, active: false, display_name: null, file_name: null },
      static_image: { ...SETUP.static_image, configured: false, active: false, display_name: null, file_name: null },
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(emptySetup);
      if (command === 'get_character_settings') return Promise.resolve({ ...SETTINGS, active_character_id: null });
      if (command === 'get_character_manifest') return Promise.reject(new Error('selection_required'));
      return Promise.resolve(undefined);
    });
    render(<CharacterPanel />);

    expect(await screen.findByRole('heading', { name: 'Live2D' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '静止画' })).toBeInTheDocument();
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
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
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
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
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
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
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
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
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
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
      if (command === 'get_character_manifest') return Promise.reject(new Error('no character model'));
      return Promise.resolve(SETTINGS);
    });
    render(<CharacterPanel />);

    expect(await screen.findByRole('alert')).toHaveTextContent('キャラクターモデルを読み込めません');
  });

  it('reloads the panel when retry is clicked after a failure', async () => {
    let manifestAttempts = 0;
    invokeMock.mockImplementation((command: string) => {
      if (command === 'get_character_setup') return Promise.resolve(SETUP);
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
