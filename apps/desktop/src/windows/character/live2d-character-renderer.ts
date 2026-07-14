import type { CharacterRendererDto } from '@parallel-world/contracts';
import type { Live2DControllerState, ModelSource } from '@parallel-world/live2d-runtime';
import type { CharacterRenderer } from './character-renderer';
import { createModelSource } from './model-source';

type Live2DRendererDto = Extract<CharacterRendererDto, { kind: 'live2d' }>;

/** Narrow controller surface so the adapter can be tested without loading Cubism. */
export interface Live2DControllerLike {
  readonly state: Live2DControllerState;
  attach(canvas: HTMLCanvasElement): Promise<void>;
  loadModel(source: ModelSource): Promise<void>;
  setExpression(name: string): boolean;
  startMotion(group: string): boolean;
  setLipSyncValue(level: number): boolean;
  resize(width: number, height: number, dpr: number): void;
  hitTest(x: number, y: number): boolean;
  dispose(): void;
}

export class Live2DCharacterRenderer implements CharacterRenderer {
  readonly kind = 'live2d' as const;
  readonly #canvas: HTMLCanvasElement;
  readonly #controller: Live2DControllerLike;
  readonly #convertFileSrc: (path: string) => string;
  #disposed = false;

  constructor(
    canvas: HTMLCanvasElement,
    controller: Live2DControllerLike,
    convertFileSrc: (path: string) => string,
  ) {
    this.#canvas = canvas;
    this.#controller = controller;
    this.#convertFileSrc = convertFileSrc;
  }

  async load(renderer: CharacterRendererDto): Promise<void> {
    if (renderer.kind !== 'live2d') {
      throw new Error('Live2DCharacterRenderer requires a live2d manifest');
    }
    this.#ensureActive();
    if (this.#controller.state === 'idle') {
      await this.#awaitWhileActive(this.#controller.attach(this.#canvas));
    }
    // Recheck in the outer continuation as disposal can occur after the
    // attach guard resolves but before this load continuation resumes.
    this.#ensureActive();
    await this.#awaitWhileActive(
      this.#controller.loadModel(
        createModelSource(renderer.model_path, this.#convertFileSrc),
      ),
    );
    // The helper's successful check and this outer continuation are distinct
    // microtasks. Reject if disposal landed in that final scheduling gap.
    this.#ensureActive();
  }

  setExpression(name: string): boolean {
    return !this.#disposed && this.#controller.setExpression(name);
  }

  startMotion(group: string): boolean {
    return !this.#disposed && this.#controller.startMotion(group);
  }

  setAudioLevel(level: number): void {
    if (!this.#disposed) this.#controller.setLipSyncValue(level);
  }

  reactToSpeechStart(_turnId: number): boolean {
    return false;
  }

  resetSpeechReaction(): void {}

  resize(width: number, height: number, dpr: number): void {
    if (!this.#disposed) this.#controller.resize(width, height, dpr);
  }

  hitTest(x: number, y: number): boolean {
    return !this.#disposed && this.#controller.hitTest(x, y);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#controller.dispose();
  }

  #ensureActive(): void {
    if (this.#disposed) throw new Error('Live2DCharacterRenderer is disposed');
  }

  async #awaitWhileActive(operation: Promise<void>): Promise<void> {
    try {
      await operation;
    } catch (error) {
      if (this.#disposed) this.#controller.dispose();
      throw error;
    }
    if (this.#disposed) {
      // The underlying controller can transition back to ready/model-loaded
      // after dispose when an in-flight runtime promise settles. Re-dispose
      // that late state before aborting the adapter load.
      this.#controller.dispose();
      throw new Error('Live2DCharacterRenderer is disposed');
    }
  }
}

export type { Live2DRendererDto };
