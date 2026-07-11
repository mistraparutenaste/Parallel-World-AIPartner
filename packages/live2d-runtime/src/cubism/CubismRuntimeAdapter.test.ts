import { describe, expect, it, vi } from "vitest";
import { CubismRuntimeAdapter, Live2DError, type CubismR5Bridge } from "../index.js";

function canvasWith(context: WebGLRenderingContext | null): HTMLCanvasElement {
  return { getContext: vi.fn(() => context) } as unknown as HTMLCanvasElement;
}

const model3 = "http://localhost/.dev-assets/live2d/models/live2d-mark/Mark.model3.json";

describe("CubismRuntimeAdapter", () => {
  it("implements the controller adapter path and resolves model3 beside the injected manifest", async () => {
    const handle = { update: vi.fn(), draw: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), resize: vi.fn(), dispose: vi.fn() };
    const bridge: CubismR5Bridge = { createModelFromModel3: vi.fn(async () => handle) };
    const canvas = canvasWith({} as WebGLRenderingContext);
    const adapter = new CubismRuntimeAdapter({ globalObject: { Live2DCubismCore: {} }, bridge, baseUrl: "http://localhost/app/" });
    adapter.mount(canvas);
    await adapter.loadModel({ modelId: "mark", manifestUrl: "assets/mark/character.json" }, {
      schemaVersion: 1, id: "mark", model3: "Mark.model3.json", motions: { Idle: [0] }, expressions: {},
    });
    expect(bridge.createModelFromModel3).toHaveBeenCalledWith(expect.objectContaining({
      model3JsonUrl: "http://localhost/app/assets/mark/Mark.model3.json",
    }));
  });
  it("rejects when Cubism Core global is absent", async () => {
    const adapter = new CubismRuntimeAdapter({ globalObject: {}, bridge: {} as CubismR5Bridge });
    await expect(adapter.createModel(canvasWith({} as WebGLRenderingContext), model3))
      .rejects.toEqual(expect.objectContaining<Partial<Live2DError>>({ code: "core-unavailable" }));
  });

  it("rejects when WebGL is unavailable", async () => {
    const adapter = new CubismRuntimeAdapter({ globalObject: { Live2DCubismCore: {} }, bridge: {} as CubismR5Bridge });
    await expect(adapter.createModel(canvasWith(null), model3))
      .rejects.toEqual(expect.objectContaining<Partial<Live2DError>>({ code: "webgl-unavailable" }));
  });

  it("rejects when the Cubism R5 Framework bridge is absent", async () => {
    const adapter = new CubismRuntimeAdapter({ globalObject: { Live2DCubismCore: {} } });
    await expect(adapter.createModel(canvasWith({} as WebGLRenderingContext), model3))
      .rejects.toEqual(expect.objectContaining<Partial<Live2DError>>({ code: "framework-unavailable" }));
  });

  it("uses the official model3 entry point and delegates lifecycle to the R5 bridge", async () => {
    const handle = { update: vi.fn(), draw: vi.fn(), playMotion: vi.fn(), setExpression: vi.fn(), resize: vi.fn(), dispose: vi.fn() };
    const bridge: CubismR5Bridge = { createModelFromModel3: vi.fn(async () => handle) };
    const gl = {} as WebGLRenderingContext;
    const canvas = canvasWith(gl);
    const adapter = new CubismRuntimeAdapter({ globalObject: { Live2DCubismCore: {} }, bridge });
    const result = await adapter.createModel(canvas, model3);
    expect(bridge.createModelFromModel3).toHaveBeenCalledWith({ canvas, gl, model3JsonUrl: model3 });
    expect(result).toBe(handle);
  });

  it.each(["https://example.com/model.model3.json", "data:application/json,{}", "/not-model.json"])(
    "rejects unsafe or non-model3 source %s", async (url) => {
      const adapter = new CubismRuntimeAdapter({ globalObject: { Live2DCubismCore: {} }, bridge: {} as CubismR5Bridge });
      await expect(adapter.createModel(canvasWith({} as WebGLRenderingContext), url))
        .rejects.toEqual(expect.objectContaining<Partial<Live2DError>>({ code: "invalid-model-source" }));
    },
  );
});
