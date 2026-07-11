/**
 * Picks the motion group used for the idle loop: the canonical
 * `Idle` group, any case-insensitive `idle` match, or the first
 * group as a last resort.
 */
export function resolveIdleGroup(groups: readonly string[]): string | null {
  if (groups.length === 0) {
    return null;
  }
  if (groups.includes('Idle')) {
    return 'Idle';
  }
  const caseInsensitive = groups.find(
    (group) => group.toLowerCase() === 'idle',
  );
  return caseInsensitive ?? groups[0];
}
