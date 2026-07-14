import type { CharacterRendererDto } from '@parallel-world/contracts';
import type { CharacterRenderer } from './character-renderer';
import {
  Live2DCharacterRenderer,
  type Live2DControllerLike,
} from './live2d-character-renderer';
import {
  StaticImageCharacterRenderer,
  defaultStaticImageDependencies,
  type StaticImageRendererDependencies,
} from './static-image-character-renderer';

export interface CharacterRendererFactoryDependencies {
  readonly canvas: HTMLCanvasElement;
  readonly convertFileSrc: (path: string) => string;
  readonly createLive2DController?: () => Live2DControllerLike;
  readonly staticImage?: Partial<Omit<StaticImageRendererDependencies, 'convertFileSrc'>>;
}

/** Selects only from the tagged renderer kind; static selection never touches Cubism. */
export function createCharacterRenderer(
  renderer: CharacterRendererDto,
  dependencies: CharacterRendererFactoryDependencies,
): CharacterRenderer {
  switch (renderer.kind) {
    case 'static_image': {
      const defaults = defaultStaticImageDependencies(dependencies.convertFileSrc);
      return new StaticImageCharacterRenderer(dependencies.canvas, {
        ...defaults,
        ...dependencies.staticImage,
        convertFileSrc: dependencies.convertFileSrc,
      });
    }
    case 'live2d': {
      const createController = dependencies.createLive2DController;
      if (createController === undefined) {
        throw new Error('createLive2DController is required for a live2d renderer');
      }
      return new Live2DCharacterRenderer(
        dependencies.canvas,
        createController(),
        dependencies.convertFileSrc,
      );
    }
    default: {
      const unsupported = renderer as { kind: string };
      throw new Error(`unsupported character renderer kind: ${unsupported.kind}`);
    }
  }
}
