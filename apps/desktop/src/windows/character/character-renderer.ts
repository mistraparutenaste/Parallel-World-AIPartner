import type { CharacterRendererDto } from '@parallel-world/contracts';

/** React-independent drawing boundary shared by all character renderers. */
export interface CharacterRenderer {
  readonly kind: CharacterRendererDto['kind'];
  load(renderer: CharacterRendererDto): Promise<void>;
  setExpression(name: string): boolean;
  startMotion(group: string): boolean;
  setAudioLevel(level: number): void;
  reactToSpeechStart(turnId: number): boolean;
  resetSpeechReaction(): void;
  resize(width: number, height: number, dpr: number): void;
  hitTest(x: number, y: number): boolean;
  dispose(): void;
}
