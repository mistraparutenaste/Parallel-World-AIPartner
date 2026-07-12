import { describe, expect, it } from 'vitest';
import { ExternalLipSyncProvider } from './external-lip-sync-provider';

describe('ExternalLipSyncProvider', () => {
  it('clamps values into the 0..1 parameter range', () => {
    const provider = new ExternalLipSyncProvider();
    expect(provider.getParameter()).toBe(0);

    provider.setValue(0.4);
    expect(provider.getParameter()).toBe(0.4);

    provider.setValue(1.7);
    expect(provider.getParameter()).toBe(1);

    provider.setValue(-0.2);
    expect(provider.getParameter()).toBe(0);

    provider.setValue(Number.NaN);
    expect(provider.getParameter()).toBe(0);
  });

  it('always reports a successful update', () => {
    expect(new ExternalLipSyncProvider().update()).toBe(true);
  });
});
