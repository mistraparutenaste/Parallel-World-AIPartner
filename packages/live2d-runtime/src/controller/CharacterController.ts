import {
  Live2DError,
  type CharacterController,
  type CharacterCleanupPhase,
  type CharacterManifestLoader,
  type CharacterModelHandle,
  type CharacterModelSource,
  type CharacterRuntimeAdapter,
  type CharacterRuntimeEnvironment,
  type CharacterRuntimeStatus,
  type CharacterViewport,
} from "../contracts.js";
import { assertSafeRelativePath, parseCharacterManifest, type CharacterManifest } from "../manifest/CharacterManifest.js";

export interface CharacterControllerDependencies {
  adapter: CharacterRuntimeAdapter;
  environment?: CharacterRuntimeEnvironment;
  loadManifest?: CharacterManifestLoader;
  /** Subscriber exceptions are isolated from runtime state changes and reported here. */
  onSubscriberError?: (error: unknown, status: CharacterRuntimeStatus) => void;
  /** Cleanup is best-effort/no-throw; every failure is reported individually here. */
  onCleanupError?: (error: unknown, phase: CharacterCleanupPhase) => void;
}

const MAX_BACKING_DIMENSION = 16_384;
const MAX_BACKING_PIXELS = 268_435_456;

function defaultEnvironment(): CharacterRuntimeEnvironment {
  return {
    requestAnimationFrame: (callback) => globalThis.requestAnimationFrame(callback),
    cancelAnimationFrame: (id) => globalThis.cancelAnimationFrame(id),
    addResizeListener(listener) {
      globalThis.addEventListener("resize", listener);
      return () => globalThis.removeEventListener("resize", listener);
    },
  };
}

async function defaultManifestLoader(source: CharacterModelSource): Promise<CharacterManifest> {
  let response: Response;
  try {
    response = await fetch(source.manifestUrl, { credentials: "same-origin" });
  } catch (cause) {
    throw new Live2DError("fetch-failed", "manifest fetch failed", { cause });
  }
  if (!response.ok) throw new Live2DError("fetch-failed", `manifest fetch returned ${response.status}`);
  try {
    return parseCharacterManifest(await response.json());
  } catch (cause) {
    if (cause instanceof Live2DError) throw cause;
    throw new Live2DError("invalid-manifest", "manifest is not valid JSON", { cause });
  }
}

function validateSource(source: CharacterModelSource): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(source.modelId)) {
    throw new Live2DError("invalid-model-source", "invalid modelId");
  }
  try {
    assertSafeRelativePath(source.manifestUrl, "manifestUrl");
  } catch (cause) {
    throw new Live2DError("invalid-model-source", "invalid manifestUrl", { cause });
  }
}

function mapLoadError(cause: unknown): Live2DError {
  if (cause instanceof Live2DError) return cause;
  return new Live2DError("model-load-failed", "model loading failed", { cause });
}

export function createCharacterController(dependencies: CharacterControllerDependencies): CharacterController {
  const environment = dependencies.environment ?? defaultEnvironment();
  const loadManifest = dependencies.loadManifest ?? defaultManifestLoader;
  let status: CharacterRuntimeStatus = { kind: "idle" };
  let lifecycle: "idle" | "mounting" | "mounted" | "disposed" = "idle";
  let loadGeneration = 0;
  let canvas: HTMLCanvasElement | undefined;
  let model: CharacterModelHandle | undefined;
  let manifest: CharacterManifest | undefined;
  let frameId: number | undefined;
  let previousTimestamp: number | undefined;
  let removeResizeListener: (() => void) | undefined;
  const listeners = new Set<(next: CharacterRuntimeStatus) => void>();
  const isDisposed = () => lifecycle === "disposed";
  const cleanup = (phase: CharacterCleanupPhase, operation: (() => void) | undefined) => {
    if (!operation) return;
    try {
      operation();
    } catch (error) {
      try {
        dependencies.onCleanupError?.(error, phase);
      } catch {
        // Cleanup and its remaining steps must survive reporter failures too.
      }
    }
  };

  const publish = (next: CharacterRuntimeStatus) => {
    status = next;
    for (const listener of [...listeners]) {
      try {
        listener(next);
      } catch (error) {
        try {
          dependencies.onSubscriberError?.(error, next);
        } catch {
          // Error reporting must never corrupt controller state or remaining notifications.
        }
      }
    }
  };

  const frame = (timestamp: number) => {
    if (lifecycle === "disposed") return;
    const delta = previousTimestamp === undefined ? 0 : Math.max(0, (timestamp - previousTimestamp) / 1_000);
    previousTimestamp = timestamp;
    model?.update(delta);
    model?.draw();
    frameId = environment.requestAnimationFrame(frame);
  };

  const assertUsable = () => {
    if (lifecycle === "disposed") throw new Live2DError("disposed");
    if (lifecycle !== "mounted") throw new Live2DError("not-mounted");
  };

  const assertReady = (): { model: CharacterModelHandle; manifest: CharacterManifest } => {
    if (lifecycle === "disposed") throw new Live2DError("disposed");
    if (!model || !manifest || status.kind !== "ready") throw new Live2DError("not-ready");
    return { model, manifest };
  };

  return {
    async mount(nextCanvas) {
      if (lifecycle === "disposed") throw new Live2DError("disposed");
      if (lifecycle === "mounting") throw new Live2DError("mount-in-progress");
      if (lifecycle === "mounted") throw new Live2DError("not-mounted", "controller is already mounted");
      lifecycle = "mounting";
      canvas = nextCanvas;
      let localRemoveResizeListener: (() => void) | undefined;
      let localFrameId: number | undefined;
      try {
        await dependencies.adapter.mount(nextCanvas);
        if (isDisposed()) {
          throw new Live2DError("disposed");
        }
        localRemoveResizeListener = environment.addResizeListener(() => {
          if (canvas && lifecycle === "mounted") {
            this.resize({
              cssWidth: canvas.clientWidth,
              cssHeight: canvas.clientHeight,
              devicePixelRatio: globalThis.devicePixelRatio || 1,
            });
          }
        });
        localFrameId = environment.requestAnimationFrame(frame);
        removeResizeListener = localRemoveResizeListener;
        frameId = localFrameId;
        lifecycle = "mounted";
      } catch (cause) {
        const externallyDisposed = isDisposed();
        if (localFrameId !== undefined) {
          const frameToCancel = localFrameId;
          cleanup("cancel-animation-frame", () => environment.cancelAnimationFrame(frameToCancel));
        }
        cleanup("remove-resize-listener", localRemoveResizeListener);
        if (externallyDisposed) {
          cleanup("dispose-adapter", () => dependencies.adapter.dispose());
        } else {
          lifecycle = "disposed";
          canvas = undefined;
          cleanup("dispose-adapter", () => dependencies.adapter.dispose());
        }
        if (externallyDisposed) {
          throw new Live2DError("disposed", "controller disposed during mount", { cause });
        }
        if (cause instanceof Live2DError) throw cause;
        throw new Live2DError("mount-failed", "runtime mount failed", { cause });
      }
    },

    async loadModel(source) {
      assertUsable();
      validateSource(source);
      const generation = ++loadGeneration;
      const previousModel = model;
      model = undefined;
      manifest = undefined;
      cleanup("dispose-model", previousModel ? () => previousModel.dispose() : undefined);
      publish({ kind: "loading", modelId: source.modelId });
      try {
        const parsed = parseCharacterManifest(await loadManifest(source));
        if (isDisposed()) throw new Live2DError("disposed");
        if (generation !== loadGeneration) throw new Live2DError("superseded");
        if (parsed.id !== source.modelId) throw new Live2DError("invalid-manifest", "manifest id does not match modelId");
        const loaded = await dependencies.adapter.loadModel(source, parsed);
        if (isDisposed() || generation !== loadGeneration) {
          cleanup("dispose-model", () => loaded.dispose());
          throw new Live2DError(isDisposed() ? "disposed" : "superseded");
        }
        model = loaded;
        manifest = parsed;
        publish({ kind: "ready", modelId: source.modelId });
      } catch (cause) {
        const error = isDisposed()
          ? new Live2DError("disposed", "controller disposed during model loading", { cause })
          : generation !== loadGeneration
            ? new Live2DError("superseded", "model load was superseded", { cause })
            : mapLoadError(cause);
        if (error.code === "disposed" || error.code === "superseded") throw error;
        publish({ kind: "failed", modelId: source.modelId, code: error.code });
        throw error;
      }
    },

    async playMotion(group, index) {
      const ready = assertReady();
      const indexes = Object.hasOwn(ready.manifest.motions, group)
        ? ready.manifest.motions[group]
        : undefined;
      if (!indexes || (index !== undefined && !indexes.includes(index))) {
        throw new Live2DError("unknown-motion");
      }
      try {
        await ready.model.playMotion(group, index);
      } catch (cause) {
        if (cause instanceof Live2DError) throw cause;
        throw new Live2DError("motion-failed", "motion playback failed", { cause });
      }
    },

    async setExpression(id) {
      const ready = assertReady();
      if (!Object.hasOwn(ready.manifest.expressions, id)) throw new Live2DError("unknown-expression");
      try {
        await ready.model.setExpression(id);
      } catch (cause) {
        if (cause instanceof Live2DError) throw cause;
        throw new Live2DError("expression-failed", "expression update failed", { cause });
      }
    },

    resize(viewport: CharacterViewport) {
      assertUsable();
      if (![viewport.cssWidth, viewport.cssHeight, viewport.devicePixelRatio].every(Number.isFinite) ||
        viewport.cssWidth < 0 || viewport.cssHeight < 0 || viewport.devicePixelRatio <= 0) {
        throw new Live2DError("invalid-viewport", "invalid viewport");
      }
      const width = Math.round(viewport.cssWidth * viewport.devicePixelRatio);
      const height = Math.round(viewport.cssHeight * viewport.devicePixelRatio);
      if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) ||
        width > MAX_BACKING_DIMENSION || height > MAX_BACKING_DIMENSION ||
        width * height > MAX_BACKING_PIXELS) {
        throw new Live2DError("invalid-viewport", "viewport backing size exceeds safe limits");
      }
      if (canvas) {
        canvas.width = width;
        canvas.height = height;
      }
      model?.resize(width, height);
    },

    dispose() {
      if (lifecycle === "disposed") return;
      const mountIsPending = lifecycle === "mounting";
      lifecycle = "disposed";
      loadGeneration += 1;
      const frameToCancel = frameId;
      const resizeListenerToRemove = removeResizeListener;
      const modelToDispose = model;
      listeners.clear();
      frameId = undefined;
      removeResizeListener = undefined;
      model = undefined;
      manifest = undefined;
      canvas = undefined;
      previousTimestamp = undefined;
      if (frameToCancel !== undefined) {
        cleanup("cancel-animation-frame", () => environment.cancelAnimationFrame(frameToCancel));
      }
      cleanup("remove-resize-listener", resizeListenerToRemove);
      cleanup("dispose-model", modelToDispose ? () => modelToDispose.dispose() : undefined);
      if (!mountIsPending) cleanup("dispose-adapter", () => dependencies.adapter.dispose());
    },

    subscribe(listener) {
      if (lifecycle === "disposed") throw new Live2DError("disposed");
      listeners.add(listener);
      try {
        listener(status);
      } catch (error) {
        try {
          dependencies.onSubscriberError?.(error, status);
        } catch {
          // See publish(): reporter failures are isolated too.
        }
      }
      let active = true;
      return () => {
        if (!active) return;
        active = false;
        listeners.delete(listener);
      };
    },
  };
}
