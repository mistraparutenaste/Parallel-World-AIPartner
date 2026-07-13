import { describe, expect, it, vi } from 'vitest';
import type { CharacterController } from '@parallel-world/live2d-runtime';
import type { CharacterPresentationSettingsDto } from '@parallel-world/contracts';
import { bindCharacterPresentation, isCurrentCharacterPresentation } from './characterPresentation';

const value = (overrides: Partial<CharacterPresentationSettingsDto> = {}): CharacterPresentationSettingsDto => ({ schema_version: 1, revision: 1, model_id: 'epsilon-free', expression_id: 'Smile', motion_group: 'Tap', motion_index: 2, click_through: false, ...overrides });
const controller = (): CharacterController => ({ mount: vi.fn(), loadModel: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), resize: vi.fn(), dispose: vi.fn(), subscribe: vi.fn(() => vi.fn()) });

describe('character presentation client', () => {
  it('rejects stale and structurally invalid event payloads', () => {
    expect(isCurrentCharacterPresentation(value())).toBe(true);
    expect(isCurrentCharacterPresentation(value({ schema_version: 0 }))).toBe(false);
    expect(isCurrentCharacterPresentation(value({ model_id: 'unknown' }))).toBe(false);
    expect(isCurrentCharacterPresentation(value({ expression_id: 'unknown' }))).toBe(false);
    expect(isCurrentCharacterPresentation(value({ motion_group: 'Tap', motion_index: 99 }))).toBe(false);
    expect(isCurrentCharacterPresentation({ schema_version: 1, model_id: 'mark' })).toBe(false);
  });
  it('gets initial state then applies only current events to the controller', async () => {
    const target = controller(); let listener: ((value: unknown) => void) | undefined;
    const unlisten = vi.fn();
    const binding = bindCharacterPresentation(target, {
      get: async () => value(),
      listen: async next => { listener = next; return unlisten; },
    });
    await binding.ready;
    expect(target.loadModel).toHaveBeenCalledWith(expect.objectContaining({ modelId: 'epsilon-free' }));
    expect(target.setExpression).toHaveBeenCalledWith('Smile');
    expect(target.playMotion).toHaveBeenCalledWith('Tap', 2);
    listener?.(value({ schema_version: 0, expression_id: 'Angry' }));
    await Promise.resolve();
    expect(target.setExpression).not.toHaveBeenCalledWith('Angry');
    listener?.(value({ revision: 2, expression_id: 'Angry', motion_group: 'Idle', motion_index: 0 }));
    await binding.flush();
    expect(target.loadModel).toHaveBeenCalledTimes(1);
    expect(target.setExpression).toHaveBeenCalledWith('Angry');
    expect(target.playMotion).toHaveBeenLastCalledWith('Idle', 0);
    binding.dispose(); expect(unlisten).toHaveBeenCalledOnce();
  });

  it('keeps a newer event when an older snapshot resolves later', async () => {
    const target = controller(); let resolveGet!: (value: unknown) => void; let listener: ((value: unknown) => void) | undefined;
    const binding = bindCharacterPresentation(target, {
      get: () => new Promise(resolve => { resolveGet = resolve; }),
      listen: async next => { listener = next; return vi.fn(); },
    });
    await Promise.resolve();
    listener?.(value({ revision: 3, expression_id: 'Angry' }));
    resolveGet(value({ revision: 2, expression_id: 'Sad' }));
    await binding.ready; await binding.flush();
    expect(target.setExpression).toHaveBeenLastCalledWith('Angry');
    expect(target.setExpression).not.toHaveBeenCalledWith('Sad');
  });

  it('immediately removes a listener that resolves after disposal', async () => {
    let resolveListen!: (unlisten: () => void) => void; const unlisten = vi.fn();
    const binding = bindCharacterPresentation(controller(), { get: async () => value(), listen: () => new Promise(resolve => { resolveListen = resolve; }) });
    await Promise.resolve(); binding.dispose(); resolveListen(unlisten);
    await binding.ready;
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('removes the listener when snapshot loading fails', async () => {
    const unlisten = vi.fn();
    const binding = bindCharacterPresentation(controller(), { get: async () => { throw new Error('get failed'); }, listen: async () => unlisten });
    await expect(binding.ready).rejects.toThrow('get failed');
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('removes the listener when initial controller application fails', async () => {
    const target = controller(); target.loadModel = vi.fn(async () => { throw new Error('load failed'); }); const unlisten = vi.fn();
    const binding = bindCharacterPresentation(target, { get: async () => value(), listen: async () => unlisten });
    await expect(binding.ready).rejects.toThrow('load failed');
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('removes the listener when a later event cannot be applied', async () => {
    const target = controller(); let listener: ((value: unknown) => void) | undefined; const unlisten = vi.fn();
    const binding = bindCharacterPresentation(target, { get: async () => value(), listen: async next => { listener = next; return unlisten; } });
    await binding.ready; target.setExpression = vi.fn(async () => { throw new Error('expression failed'); });
    listener?.(value({ revision: 2, expression_id: 'Angry' }));
    await expect(binding.flush()).rejects.toThrow('expression failed');
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
