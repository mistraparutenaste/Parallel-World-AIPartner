type MatchMedia = (query: string) => Pick<MediaQueryList, 'matches'>;

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';
const HOP_KEYFRAMES: Keyframe[] = [
  { transform: 'translateY(0)' },
  { transform: 'translateY(-12px)' },
  { transform: 'translateY(0)' },
];
const HOP_OPTIONS: KeyframeAnimationOptions = {
  duration: 300,
  easing: 'cubic-bezier(.2,.8,.3,1)',
};

/** Owns the one-shot visual reaction for an actually-started speech turn. */
export class SpeechHopController {
  readonly #element: HTMLElement;
  readonly #matchMedia: MatchMedia;
  readonly #baseTransform: string;
  #lastTurnId: number | null = null;
  #animation: Animation | null = null;

  constructor(
    element: HTMLElement,
    matchMedia: MatchMedia = (query) => window.matchMedia(query),
  ) {
    this.#element = element;
    this.#matchMedia = matchMedia;
    this.#baseTransform = element.style.transform;
  }

  /** Starts one hop for a new turn and rejects duplicate chunks. */
  react(turnId: number): boolean {
    if (turnId === this.#lastTurnId) {
      return false;
    }
    this.cancel();
    this.#lastTurnId = turnId;
    if (this.#matchMedia(REDUCED_MOTION_QUERY).matches) {
      return true;
    }
    this.#animation = this.#element.animate(HOP_KEYFRAMES, HOP_OPTIONS);
    return true;
  }

  /** Stops the active hop without clearing the per-turn dedupe. */
  cancel(): void {
    this.#animation?.cancel();
    this.#animation = null;
    this.#element.style.transform = this.#baseTransform;
  }
}
