import { createReadStream, lstatSync, realpathSync, statSync } from "node:fs";
import { extname, relative, resolve } from "node:path";
import type { Plugin } from "vite";

const PREFIX = "/__live2d_dev__/";
const CONTENT_TYPES: Record<string, string> = {
  ".js": "text/javascript; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".moc3": "application/octet-stream", ".png": "image/png", ".vert": "text/plain; charset=utf-8",
  ".frag": "text/plain; charset=utf-8",
};

export type DevAssetResolution = { status: 200; file: string } | { status: 400 | 403 | 404 | 405 };

export function resolveLive2DDevAsset(root: string, rawPathname: string, method = "GET"): DevAssetResolution {
  if (method !== "GET" && method !== "HEAD") return { status: 405 };
  if (!rawPathname.startsWith(PREFIX)) return { status: 404 };
  let decoded: string;
  try { decoded = decodeURIComponent(rawPathname.slice(PREFIX.length)); }
  catch { return { status: 400 }; }
  if (!decoded || decoded.includes("\\") || decoded.includes("\0") || /%[0-9a-f]{2}/i.test(decoded)) return { status: 400 };
  const segments = decoded.split("/");
  if (segments.some(segment => !segment || segment === "." || segment === "..")) return { status: 403 };
  let realRoot: string;
  try { realRoot = realpathSync(root); } catch { return { status: 404 }; }
  const lexical = resolve(realRoot, ...segments);
  const lexicalRelative = relative(realRoot, lexical);
  if (lexicalRelative.startsWith("..") || resolve(realRoot, lexicalRelative) !== lexical) return { status: 403 };
  let current = realRoot;
  try {
    for (const segment of segments) {
      current = resolve(current, segment);
      if (lstatSync(current).isSymbolicLink()) return { status: 403 };
    }
    if (!statSync(lexical).isFile()) return { status: 404 };
    const realFile = realpathSync(lexical);
    const realRelative = relative(realRoot, realFile);
    if (realRelative.startsWith("..") || realRelative === "" || realFile !== resolve(realRoot, realRelative)) return { status: 403 };
    return { status: 200, file: realFile };
  } catch { return { status: 404 }; }
}

/** Serves ignored, verified assets only in `vite serve`; never in build. */
export function live2dDevAssets(root: string): Plugin {
  return {
    name: "parallel-world-live2d-dev-assets", apply: "serve",
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        let pathname: string;
        try { pathname = new URL(request.url ?? "/", "http://localhost").pathname; }
        catch { response.statusCode = 400; response.end(); return; }
        if (!pathname.startsWith(PREFIX)) return next();
        const result = resolveLive2DDevAsset(root, pathname, request.method);
        if (result.status !== 200) { response.statusCode = result.status; response.end(); return; }
        response.setHeader("Content-Type", CONTENT_TYPES[extname(result.file)] ?? "application/octet-stream");
        response.setHeader("Cache-Control", "no-store");
        if (request.method === "HEAD") { response.statusCode = 200; response.end(); return; }
        createReadStream(result.file).pipe(response);
      });
    },
    transformIndexHtml: { order: "pre", handler(html, context) {
      if (!context.filename.endsWith("character.html")) return html;
      return { html, tags: [
        { tag: "script", attrs: { src: `${PREFIX}core/live2dcubismcore.min.js` }, injectTo: "head-prepend" },
        { tag: "script", attrs: { src: `${PREFIX}framework-build/demo/parallel-world-cubism-r5-bridge.js`, type: "module" }, injectTo: "head-prepend" },
      ] };
    } },
  };
}
