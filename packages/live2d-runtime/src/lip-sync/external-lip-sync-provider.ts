import { IParameterProvider } from '../../vendor/framework/src/motion/iparameterprovider';

/**
 * Feeds an externally computed audio level (0..1) into the framework's
 * CubismLipSyncUpdater. The playback layer analyses the WAV amplitude
 * per frame and pushes the value here (基本設計 9章).
 */
export class ExternalLipSyncProvider extends IParameterProvider {
  #value = 0;

  setValue(value: number): void {
    this.#value = Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
  }

  override update(): boolean {
    return true;
  }

  override getParameter(): number {
    return this.#value;
  }
}
