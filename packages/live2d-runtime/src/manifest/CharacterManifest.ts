import { Live2DError } from "../contracts.js";

export interface CharacterManifest {
  schemaVersion: 1;
  id: string;
  model3: string;
  motions: Readonly<Record<string, readonly number[]>>;
  expressions: Readonly<Record<string, string>>;
}

const TOP_LEVEL_KEYS = new Set(["schemaVersion", "id", "model3", "motions", "expressions"]);
const SAFE_ID = /^[A-Za-z0-9][A-Za-z0-9_.-]*$/;
const RESERVED_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function invalid(message: string): never {
  throw new Live2DError("invalid-manifest", message);
}

function isPlainOwnRecord(value: unknown): value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

export function assertSafeRelativePath(value: unknown, field: string): string {
  if (typeof value !== "string" || value.length === 0 || value.includes("\\") || value.includes("\0")) {
    return invalid(`${field} must be a non-empty forward-slash relative path`);
  }
  let decoded = value;
  for (let depth = 0; depth < 8; depth += 1) {
    let next: string;
    try {
      next = decodeURIComponent(decoded);
    } catch {
      return invalid(`${field} contains invalid encoding`);
    }
    if (next === decoded) break;
    decoded = next;
    if (depth === 7) return invalid(`${field} is excessively encoded`);
  }
  if (decoded.includes("\\") || decoded.includes("\0") || decoded.includes("?") ||
    decoded.includes("#") || decoded.includes("%")) {
    return invalid(`${field} contains a forbidden separator or URL component`);
  }
  if (decoded.startsWith("/") || /^[A-Za-z]:/.test(decoded) || /^[A-Za-z][A-Za-z0-9+.-]*:/.test(decoded)) {
    return invalid(`${field} must not be absolute or use a URL scheme`);
  }
  const segments = decoded.split("/");
  if (segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
    return invalid(`${field} contains an unsafe segment`);
  }
  return decoded;
}

export function parseCharacterManifest(input: unknown): CharacterManifest {
  if (!isPlainOwnRecord(input)) invalid("manifest must be a plain object");
  for (const key of TOP_LEVEL_KEYS) {
    if (!Object.hasOwn(input, key)) invalid(`missing own key: ${key}`);
  }
  for (const key of Object.keys(input)) {
    if (!TOP_LEVEL_KEYS.has(key)) invalid(`unknown key: ${key}`);
  }
  if (input.schemaVersion !== 1) invalid("unsupported schemaVersion");
  if (typeof input.id !== "string" || !SAFE_ID.test(input.id)) invalid("invalid id");
  const model3 = assertSafeRelativePath(input.model3, "model3");

  if (!isPlainOwnRecord(input.motions)) invalid("motions must be a plain object");
  const motions: Record<string, readonly number[]> = {};
  for (const [group, indexes] of Object.entries(input.motions)) {
    if (!SAFE_ID.test(group) || RESERVED_KEYS.has(group) || !Array.isArray(indexes) || indexes.length === 0 ||
      indexes.some((index) => !Number.isSafeInteger(index) || (index as number) < 0)) {
      invalid(`invalid motion group: ${group}`);
    }
    motions[group] = [...(indexes as number[])];
  }

  if (!isPlainOwnRecord(input.expressions)) invalid("expressions must be a plain object");
  const expressions: Record<string, string> = {};
  for (const [id, path] of Object.entries(input.expressions)) {
    if (!SAFE_ID.test(id) || RESERVED_KEYS.has(id)) invalid(`invalid expression id: ${id}`);
    expressions[id] = assertSafeRelativePath(path, `expressions.${id}`);
  }
  return { schemaVersion: 1, id: input.id, model3, motions, expressions };
}
