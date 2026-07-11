/** URL helpers for resolving model3.json resource references. */

/** Directory URL (with trailing slash) that contains the model3.json. */
export function modelBaseUrl(modelUrl: string): string {
  const slash = modelUrl.lastIndexOf('/');
  return slash < 0 ? '' : modelUrl.slice(0, slash + 1);
}

/**
 * Joins a base directory with a relative resource path from a
 * model3.json, percent-encoding each segment.
 */
export function resolveResourceUrl(baseUrl: string, relativePath: string): string {
  const encoded = relativePath
    .split('/')
    .map((segment) => encodeURIComponent(segment))
    .join('/');
  return baseUrl + encoded;
}
