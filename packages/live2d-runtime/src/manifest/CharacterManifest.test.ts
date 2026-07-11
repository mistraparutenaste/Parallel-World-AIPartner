import { describe, expect, it } from "vitest";
import { Live2DError, parseCharacterManifest } from "../index.js";

const valid = {
  schemaVersion: 1,
  id: "mark",
  model3: "Mark.model3.json",
  motions: { Idle: [0, 1] },
  expressions: { smile: "expressions/smile.exp3.json" },
};

describe("parseCharacterManifest", () => {
  it("parses a strict manifest", () => {
    expect(parseCharacterManifest(valid)).toEqual(valid);
  });

  it.each([
    ["unknown key", { ...valid, extra: true }],
    ["path traversal", { ...valid, model3: "../Mark.model3.json" }],
    ["encoded traversal", { ...valid, model3: "%2e%2e/Mark.model3.json" }],
    ["absolute path", { ...valid, model3: "/Mark.model3.json" }],
    ["remote source", { ...valid, model3: "https://example.test/Mark.model3.json" }],
    ["unknown nested key", { ...valid, motions: { Idle: [0], nope: "bad" } }],
    ["encoded backslash", { ...valid, model3: "%5c..%5csecret" }],
    ["encoded NUL", { ...valid, model3: "model%00.json" }],
    ["double encoded traversal", { ...valid, model3: "%252e%252e/secret" }],
    ["query", { ...valid, model3: "Mark.model3.json?remote=1" }],
    ["fragment", { ...valid, model3: "Mark.model3.json#remote" }],
  ])("rejects %s", (_name, input) => {
    expect(() => parseCharacterManifest(input)).toThrowError(
      expect.objectContaining<Partial<Live2DError>>({ code: "invalid-manifest" }),
    );
  });

  it.each([
    ["inherited manifest", Object.assign(Object.create({ schemaVersion: 1 }), valid)],
    ["class instance", Object.assign(new (class Manifest {})(), valid)],
    ["inherited motions", { ...valid, motions: Object.create({ Idle: [0] }) }],
    ["inherited expressions", { ...valid, expressions: Object.create({ smile: "smile.exp3.json" }) }],
    ["reserved motion key", { ...valid, motions: { constructor: [0] } }],
    ["reserved expression key", { ...valid, expressions: { prototype: "smile.exp3.json" } }],
  ])("rejects non-own or prototype-sensitive %s", (_name, input) => {
    expect(() => parseCharacterManifest(input)).toThrowError(
      expect.objectContaining<Partial<Live2DError>>({ code: "invalid-manifest" }),
    );
  });

  it("accepts null-prototype own records and normalizes them safely", () => {
    const input = Object.assign(Object.create(null), valid, {
      motions: Object.assign(Object.create(null), { Idle: [0] }),
      expressions: Object.assign(Object.create(null), { smile: "smile.exp3.json" }),
    });
    const parsed = parseCharacterManifest(input);
    expect(parsed.motions.Idle).toEqual([0]);
    expect(parsed.expressions.smile).toBe("smile.exp3.json");
  });
});
