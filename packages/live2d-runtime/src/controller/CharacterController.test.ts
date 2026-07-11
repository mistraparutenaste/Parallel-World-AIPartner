import { describe, expect, it, vi } from "vitest";
import {
  createCharacterController,
  Live2DError,
  type CharacterManifest,
  type CharacterModelHandle,
  type CharacterRuntimeAdapter,
  type CharacterRuntimeEnvironment,
} from "../index.js";

const manifest = (id: string): CharacterManifest => ({
  schemaVersion: 1,
  id,
  model3: `${id}.model3.json`,
  motions: { Idle: [0, 1] },
  expressions: { smile: "smile.exp3.json" },
});

function harness() {
  let frame = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const resizeListeners = new Set<() => void>();
  const handles: Array<CharacterModelHandle & {
    dispose: ReturnType<typeof vi.fn>;
    resize: ReturnType<typeof vi.fn>;
  }> = [];
  const adapter: CharacterRuntimeAdapter = {
    mount: vi.fn(),
    loadModel: vi.fn(async (_source, parsed) => {
      const handle = {
        update: vi.fn(), draw: vi.fn(), resize: vi.fn(),
        playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn(),
      };
      handles.push(handle);
      return handle;
    }),
    dispose: vi.fn(),
  };
  const environment: CharacterRuntimeEnvironment = {
    requestAnimationFrame(callback) { frames.set(++frame, callback); return frame; },
    cancelAnimationFrame(id) { frames.delete(id); },
    addResizeListener(listener) { resizeListeners.add(listener); return () => resizeListeners.delete(listener); },
  };
  const loadManifest = vi.fn(async ({ modelId }: { modelId: string }) => manifest(modelId));
  return { adapter, environment, frames, resizeListeners, handles, loadManifest };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

describe("CharacterController", () => {
  it("reports stable not-ready errors before loading", async () => {
    const h = harness();
    const controller = createCharacterController(h);
    await expect(controller.playMotion("Idle")).rejects.toMatchObject({ code: "not-ready" });
    await expect(controller.setExpression("smile")).rejects.toMatchObject({ code: "not-ready" });
  });

  it("publishes idle, loading and ready states", async () => {
    const h = harness();
    const controller = createCharacterController(h);
    const statuses: string[] = [];
    controller.subscribe((status) => statuses.push(status.kind));
    await controller.mount({ width: 0, height: 0 } as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "models/mark/character.json" });
    expect(statuses).toEqual(["idle", "loading", "ready"]);
  });

  it("disposes the old model before switching", async () => {
    const h = harness();
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    await controller.loadModel({ modelId: "epsilon", manifestUrl: "epsilon/character.json" });
    expect(h.handles[0]?.dispose).toHaveBeenCalledOnce();
    expect(h.adapter.loadModel).toHaveBeenCalledTimes(2);
  });

  it("validates motion and expression readiness against the manifest", async () => {
    const h = harness();
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    await expect(controller.playMotion("Unknown")).rejects.toMatchObject({ code: "unknown-motion" });
    await expect(controller.playMotion("toString")).rejects.toMatchObject({ code: "unknown-motion" });
    await expect(controller.playMotion("Idle", 9)).rejects.toMatchObject({ code: "unknown-motion" });
    await expect(controller.setExpression("missing")).rejects.toMatchObject({ code: "unknown-expression" });
    await expect(controller.setExpression("toString")).rejects.toMatchObject({ code: "unknown-expression" });
    await controller.playMotion("Idle", 1);
    await controller.setExpression("smile");
  });

  it("preserves an omitted motion index for adapter-side selection", async () => {
    const h = harness();
    h.loadManifest.mockResolvedValueOnce({ ...manifest("mark"), motions: { Idle: [1] } });
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    await controller.playMotion("Idle");
    expect(h.handles[0]?.playMotion).toHaveBeenCalledWith("Idle", undefined);
  });

  it("applies DPR to canvas backing size and adapter viewport", async () => {
    const h = harness();
    const controller = createCharacterController(h);
    const canvas = { width: 0, height: 0 } as HTMLCanvasElement;
    await controller.mount(canvas);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    controller.resize({ cssWidth: 320.5, cssHeight: 200.25, devicePixelRatio: 2 });
    expect([canvas.width, canvas.height]).toEqual([641, 401]);
    expect(h.handles[0]?.resize).toHaveBeenCalledWith(641, 401);
    controller.resize({ cssWidth: 0, cssHeight: 0, devicePixelRatio: 2.5 });
    expect([canvas.width, canvas.height]).toEqual([0, 0]);
  });

  it("cleans RAF, listener, model and adapter across repeated lifecycle", async () => {
    const h = harness();
    for (let index = 0; index < 2; index += 1) {
      const controller = createCharacterController(h);
      await controller.mount({} as HTMLCanvasElement);
      await controller.loadModel({ modelId: `model-${index}`, manifestUrl: `model-${index}/character.json` });
      expect(h.frames.size).toBe(1);
      expect(h.resizeListeners.size).toBe(1);
      controller.dispose();
      controller.dispose();
      expect(h.frames.size).toBe(0);
      expect(h.resizeListeners.size).toBe(0);
    }
    expect(h.handles.every((handle) => handle.dispose.mock.calls.length === 1)).toBe(true);
    expect(h.adapter.dispose).toHaveBeenCalledTimes(2);
  });

  it("continues every cleanup step when all resource disposers throw", async () => {
    const h = harness();
    const cleanupErrors: string[] = [];
    h.environment.cancelAnimationFrame = vi.fn(() => { throw new Error("cancel"); });
    h.environment.addResizeListener = vi.fn(() => () => { throw new Error("listener"); });
    h.adapter.dispose = vi.fn(() => { throw new Error("adapter"); });
    const controller = createCharacterController({
      ...h,
      onCleanupError: (_error, phase) => {
        cleanupErrors.push(phase);
        throw new Error("cleanup reporter");
      },
    });
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    h.handles[0]!.dispose = vi.fn(() => { throw new Error("model"); });

    expect(() => controller.dispose()).not.toThrow();
    expect(() => controller.dispose()).not.toThrow();
    expect(h.environment.cancelAnimationFrame).toHaveBeenCalledOnce();
    expect(h.handles[0]!.dispose).toHaveBeenCalledOnce();
    expect(h.adapter.dispose).toHaveBeenCalledOnce();
    expect(cleanupErrors).toEqual([
      "cancel-animation-frame",
      "remove-resize-listener",
      "dispose-model",
      "dispose-adapter",
    ]);
    expect(() => controller.subscribe(() => undefined)).toThrowError(
      expect.objectContaining({ code: "disposed" }),
    );
  });

  it("clears a throwing old model disposer and continues the model switch", async () => {
    const h = harness();
    const cleanupErrors: string[] = [];
    const controller = createCharacterController({
      ...h,
      onCleanupError: (_error, phase) => cleanupErrors.push(phase),
    });
    const states: string[] = [];
    controller.subscribe((state) => states.push(`${state.kind}:${"modelId" in state ? state.modelId : ""}`));
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    h.handles[0]!.dispose = vi.fn(() => { throw new Error("old model"); });

    await expect(controller.loadModel({ modelId: "epsilon", manifestUrl: "epsilon/character.json" }))
      .resolves.toBeUndefined();
    expect(h.handles[0]!.dispose).toHaveBeenCalledOnce();
    expect(h.adapter.loadModel).toHaveBeenCalledTimes(2);
    expect(states.at(-1)).toBe("ready:epsilon");
    expect(cleanupErrors).toEqual(["dispose-model"]);
  });

  it("keeps the latest concurrent load before and after adapter creation", async () => {
    const early = harness();
    const earlyManifest = deferred<CharacterManifest>();
    early.loadManifest.mockImplementation(({ modelId }) => modelId === "a" ? earlyManifest.promise : Promise.resolve(manifest("b")));
    const earlyController = createCharacterController(early);
    await earlyController.mount({} as HTMLCanvasElement);
    const earlyA = earlyController.loadModel({ modelId: "a", manifestUrl: "a/character.json" });
    await earlyController.loadModel({ modelId: "b", manifestUrl: "b/character.json" });
    earlyManifest.resolve(manifest("a"));
    await expect(earlyA).rejects.toMatchObject({ code: "superseded" });
    expect(early.adapter.loadModel).toHaveBeenCalledTimes(1);
    expect(early.handles[0]?.dispose).not.toHaveBeenCalled();

    const h = harness();
    const pendingA = deferred<CharacterModelHandle>();
    const pendingB = deferred<CharacterModelHandle>();
    const aHandle = { update: vi.fn(), draw: vi.fn(), resize: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn() };
    const bHandle = { update: vi.fn(), draw: vi.fn(), resize: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn() };
    h.adapter.loadModel = vi.fn(({ modelId }) => modelId === "a" ? pendingA.promise : pendingB.promise);
    const controller = createCharacterController(h);
    const states: string[] = [];
    controller.subscribe((state) => states.push(`${state.kind}:${"modelId" in state ? state.modelId : ""}`));
    await controller.mount({} as HTMLCanvasElement);
    const a = controller.loadModel({ modelId: "a", manifestUrl: "a/character.json" });
    await vi.waitFor(() => expect(h.adapter.loadModel).toHaveBeenCalledOnce());
    const b = controller.loadModel({ modelId: "b", manifestUrl: "b/character.json" });
    await vi.waitFor(() => expect(h.adapter.loadModel).toHaveBeenCalledTimes(2));
    pendingB.resolve(bHandle);
    await b;
    pendingA.resolve(aHandle);
    await expect(a).rejects.toMatchObject({ code: "superseded" });
    expect(aHandle.dispose).toHaveBeenCalledOnce();
    expect(bHandle.dispose).not.toHaveBeenCalled();
    expect(states.at(-1)).toBe("ready:b");

    const reverse = harness();
    const reverseA = deferred<CharacterModelHandle>();
    const reverseB = deferred<CharacterModelHandle>();
    const reverseAHandle = { update: vi.fn(), draw: vi.fn(), resize: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn() };
    const reverseBHandle = { update: vi.fn(), draw: vi.fn(), resize: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn() };
    reverse.adapter.loadModel = vi.fn(({ modelId }) => modelId === "a" ? reverseA.promise : reverseB.promise);
    const reverseController = createCharacterController(reverse);
    await reverseController.mount({} as HTMLCanvasElement);
    const reverseAPromise = reverseController.loadModel({ modelId: "a", manifestUrl: "a/character.json" });
    await vi.waitFor(() => expect(reverse.adapter.loadModel).toHaveBeenCalledOnce());
    const reverseBPromise = reverseController.loadModel({ modelId: "b", manifestUrl: "b/character.json" });
    await vi.waitFor(() => expect(reverse.adapter.loadModel).toHaveBeenCalledTimes(2));
    reverseA.resolve(reverseAHandle);
    await expect(reverseAPromise).rejects.toMatchObject({ code: "superseded" });
    expect(reverseAHandle.dispose).toHaveBeenCalledOnce();
    reverseB.resolve(reverseBHandle);
    await reverseBPromise;
    expect(reverseBHandle.dispose).not.toHaveBeenCalled();
  });

  it("does not publish a stale load failure over the latest ready model", async () => {
    const h = harness();
    const aManifest = deferred<CharacterManifest>();
    h.loadManifest.mockImplementation(({ modelId }) => modelId === "a" ? aManifest.promise : Promise.resolve(manifest("b")));
    const controller = createCharacterController(h);
    const states: string[] = [];
    controller.subscribe((state) => states.push(`${state.kind}:${"modelId" in state ? state.modelId : ""}`));
    await controller.mount({} as HTMLCanvasElement);
    const a = controller.loadModel({ modelId: "a", manifestUrl: "a/character.json" });
    await controller.loadModel({ modelId: "b", manifestUrl: "b/character.json" });
    aManifest.reject(new Error("late failure"));
    await expect(a).rejects.toMatchObject({ code: "superseded" });
    expect(states.at(-1)).toBe("ready:b");
  });

  it("disposes a handle that completes after controller disposal without publishing", async () => {
    const h = harness();
    const pendingHandle = deferred<CharacterModelHandle>();
    h.adapter.loadModel = vi.fn(() => pendingHandle.promise);
    const controller = createCharacterController(h);
    const states: string[] = [];
    controller.subscribe((state) => states.push(state.kind));
    await controller.mount({} as HTMLCanvasElement);
    const load = controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    await vi.waitFor(() => expect(h.adapter.loadModel).toHaveBeenCalledOnce());
    controller.dispose();
    const lateHandle = { update: vi.fn(), draw: vi.fn(), resize: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), dispose: vi.fn() };
    pendingHandle.resolve(lateHandle);
    await expect(load).rejects.toMatchObject({ code: "disposed" });
    expect(lateHandle.dispose).toHaveBeenCalledOnce();
    expect(states).toEqual(["idle", "loading"]);
  });

  it("rejects concurrent mount at entry and creates one RAF/listener", async () => {
    const h = harness();
    const pending = deferred<void>();
    h.adapter.mount = vi.fn(() => pending.promise);
    const controller = createCharacterController(h);
    const first = controller.mount({} as HTMLCanvasElement);
    await expect(controller.mount({} as HTMLCanvasElement)).rejects.toMatchObject({ code: "mount-in-progress" });
    pending.resolve();
    await first;
    expect(h.frames.size).toBe(1);
    expect(h.resizeListeners.size).toBe(1);
  });

  it("leaves no RAF/listener when disposed during pending mount", async () => {
    const h = harness();
    const pending = deferred<void>();
    h.adapter.mount = vi.fn(() => pending.promise);
    const controller = createCharacterController(h);
    const mounting = controller.mount({} as HTMLCanvasElement);
    controller.dispose();
    pending.resolve();
    await expect(mounting).rejects.toMatchObject({ code: "disposed" });
    expect(h.frames.size).toBe(0);
    expect(h.resizeListeners.size).toBe(0);
    expect(h.adapter.dispose).toHaveBeenCalled();
  });

  it("maps mount rejection and leaves no RAF/listener", async () => {
    const h = harness();
    h.adapter.mount = vi.fn(async () => { throw new TypeError("webgl init"); });
    const controller = createCharacterController(h);
    await expect(controller.mount({} as HTMLCanvasElement)).rejects.toMatchObject({ code: "mount-failed" });
    expect(h.frames.size).toBe(0);
    expect(h.resizeListeners.size).toBe(0);
    expect(h.adapter.dispose).toHaveBeenCalledOnce();
  });

  it("preserves the mount error when rollback cleanup also throws", async () => {
    const h = harness();
    const mountFailure = new Error("schedule frame");
    const cleanupErrors: string[] = [];
    h.environment.addResizeListener = vi.fn(() => () => { throw new Error("remove listener"); });
    h.environment.requestAnimationFrame = vi.fn(() => { throw mountFailure; });
    h.adapter.dispose = vi.fn(() => { throw new Error("adapter dispose"); });
    const controller = createCharacterController({
      ...h,
      onCleanupError: (_error, phase) => cleanupErrors.push(phase),
    });
    await expect(controller.mount({} as HTMLCanvasElement)).rejects.toMatchObject({
      code: "mount-failed",
      cause: mountFailure,
    });
    expect(cleanupErrors).toEqual(["remove-resize-listener", "dispose-adapter"]);
    expect(h.adapter.dispose).toHaveBeenCalledOnce();
  });

  it("isolates subscriber failures, reports them, and supports idempotent unsubscribe", async () => {
    const h = harness();
    const reported: unknown[] = [];
    const controller = createCharacterController({ ...h, onSubscriberError: (error, status) => reported.push([error, status.kind]) });
    const bad = controller.subscribe(() => { throw new Error("listener"); });
    const observed: string[] = [];
    const unsubscribe = controller.subscribe((state) => observed.push(state.kind));
    await controller.mount({} as HTMLCanvasElement);
    await controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    bad(); bad();
    unsubscribe(); unsubscribe();
    expect(observed).toEqual(["idle", "loading", "ready"]);
    expect(reported).toHaveLength(3);
  });

  it.each([
    [{ cssWidth: Number.MAX_VALUE, cssHeight: 1, devicePixelRatio: 2 }, "overflow"],
    [{ cssWidth: 16_385, cssHeight: 1, devicePixelRatio: 1 }, "dimension limit"],
    [{ cssWidth: 16_384, cssHeight: 16_384, devicePixelRatio: 1.01 }, "pixel limit"],
  ] as const)("rejects unsafe viewport %s", async (viewport, _reason) => {
    const h = harness();
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    expect(() => controller.resize(viewport)).toThrowError(expect.objectContaining({ code: "invalid-viewport" }));
  });

  it("maps adapter TypeError to model-load-failed and action failures to stable codes", async () => {
    const fetchHarness = harness();
    vi.stubGlobal("fetch", vi.fn(async () => { throw new TypeError("network"); }));
    const fetchController = createCharacterController({
      adapter: fetchHarness.adapter,
      environment: fetchHarness.environment,
    });
    await fetchController.mount({} as HTMLCanvasElement);
    await expect(fetchController.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" }))
      .rejects.toMatchObject({ code: "fetch-failed" });

    vi.stubGlobal("fetch", vi.fn(async () => ({
      ok: true,
      json: async () => { throw new SyntaxError("invalid JSON"); },
    })));
    const invalidJsonController = createCharacterController({
      adapter: fetchHarness.adapter,
      environment: fetchHarness.environment,
    });
    await invalidJsonController.mount({} as HTMLCanvasElement);
    await expect(invalidJsonController.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" }))
      .rejects.toMatchObject({ code: "invalid-manifest" });
    vi.unstubAllGlobals();

    const h = harness();
    h.adapter.loadModel = vi.fn(async () => { throw new TypeError("SDK bug"); });
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    await expect(controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" }))
      .rejects.toMatchObject({ code: "model-load-failed" });

    const h2 = harness();
    const controller2 = createCharacterController(h2);
    await controller2.mount({} as HTMLCanvasElement);
    await controller2.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" });
    h2.handles[0]!.playMotion = vi.fn(async () => { throw new TypeError("motion SDK bug"); });
    h2.handles[0]!.setExpression = vi.fn(async () => { throw new TypeError("expression SDK bug"); });
    await expect(controller2.playMotion("Idle")).rejects.toMatchObject({ code: "motion-failed" });
    await expect(controller2.setExpression("smile")).rejects.toMatchObject({ code: "expression-failed" });
  });

  it.each([
    [new Live2DError("invalid-manifest"), "invalid-manifest"],
    [new TypeError("loader bug"), "model-load-failed"],
    [new Error("other"), "model-load-failed"],
  ] as const)("maps load failure to %s", async (failure, code) => {
    const h = harness();
    h.loadManifest.mockRejectedValueOnce(failure);
    const controller = createCharacterController(h);
    const states: unknown[] = [];
    controller.subscribe((state) => states.push(state));
    await controller.mount({} as HTMLCanvasElement);
    await expect(controller.loadModel({ modelId: "mark", manifestUrl: "mark/character.json" }))
      .rejects.toMatchObject({ code });
    expect(states.at(-1)).toMatchObject({ kind: "failed", modelId: "mark", code });
  });

  it.each(["../character.json", "%2e%2e/character.json", "/character.json", "https://example.test/character.json"])(
    "rejects unsafe model source %s before loading",
    async (manifestUrl) => {
    const h = harness();
    const controller = createCharacterController(h);
    await controller.mount({} as HTMLCanvasElement);
    await expect(controller.loadModel({ modelId: "mark", manifestUrl }))
      .rejects.toMatchObject({ code: "invalid-model-source" });
    expect(h.loadManifest).not.toHaveBeenCalled();
    },
  );
});
