import type { ModelSource } from '@parallel-world/live2d-runtime';

/**
 * Builds a ModelSource for an absolute model3.json filesystem path.
 *
 * Relative references inside the model3.json are resolved against the
 * model's directory in path space and only then converted to an asset
 * URL — Tauri asset URLs encode the whole path as one segment, so
 * URL-space resolution would silently point at the protocol root.
 */
export function createModelSource(
  modelPath: string,
  convert: (filePath: string) => string,
): ModelSource {
  const cut = Math.max(modelPath.lastIndexOf('\\'), modelPath.lastIndexOf('/'));
  const directory = cut < 0 ? '' : modelPath.slice(0, cut);
  const separator = modelPath.includes('\\') ? '\\' : '/';
  return {
    modelUrl: convert(modelPath),
    resolveResource: (relativePath) =>
      convert(directory + separator + relativePath.replaceAll('/', separator)),
  };
}
