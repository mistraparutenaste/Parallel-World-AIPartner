import { StatusBadge } from '../../shared/components/StatusBadge';

/**
 * Transparent always-on-top character surface.
 *
 * The canvas element is reserved for the Live2D renderer (Phase 1);
 * until a model is loaded only the status badge is visible.
 */
export function CharacterWindow() {
  return (
    <main aria-label="キャラクター">
      <canvas aria-hidden="true" data-live2d-surface />
      <StatusBadge state="starting" />
    </main>
  );
}
