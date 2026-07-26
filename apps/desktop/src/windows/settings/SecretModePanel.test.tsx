import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SecretModePanel } from './SecretModePanel';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

const center = (temporary: boolean) => ({
  schema_version: 1, domains: [], memories: [], pending: [], commitments: [],
  dialogue: null, temporary, temporary_revision: temporary ? 2 : 1,
});

describe('SecretModePanel', () => {
  beforeEach(() => invokeMock.mockReset());

  it('renames and toggles temporary conversation mode', async () => {
    invokeMock
      .mockResolvedValueOnce(center(false))
      .mockResolvedValueOnce(center(true));
    render(<SecretModePanel />);
    const toggle = await screen.findByRole('switch', { name: 'シークレットモード' });
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    expect(invokeMock).toHaveBeenLastCalledWith('set_temporary_conversation', {
      temporary: true, expectedRevision: 1,
    });
  });
});
