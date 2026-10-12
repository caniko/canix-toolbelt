/** Pure structural validation and discovery; these functions grant no authority. */
import { schema, schemaDigest, type Message, type Capability } from "./types.ts";
export type { Message, Implementation, Capability } from "./types.ts";

type Definition = string | {
  type?: string;
  format?: string;
  maxBytes?: number;
  const?: number;
  enum?: readonly unknown[];
  fields?: Record<string, Definition>;
  variants?: Record<string, Definition>;
  tag?: string;
  items?: Definition;
  maxItems?: number;
  unique?: boolean;
};

const definitions = schema.definitions as Record<string, Definition>;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });

/** Only redacted machine-readable diagnostics cross the protocol boundary. */
export class ProtocolError extends Error {
  readonly code: string;
  constructor(code: string) {
    super(`execution protocol rejected: ${code}`);
    this.code = code;
  }
}

function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (object(value)) return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}

function canonicalNumbers(value: unknown): boolean {
  if (typeof value === "number") return Number.isInteger(value) && value >= 0 && value <= schema.maxJsonInteger;
  if (Array.isArray(value)) return value.every(canonicalNumbers);
  if (object(value)) return Object.values(value).every(canonicalNumbers);
  return true;
}

function matches(value: unknown, definition: Definition): boolean {
  if (typeof definition === "string") {
    return definition === "bool" ? typeof value === "boolean" : definitions[definition] !== undefined && matches(value, definitions[definition]);
  }
  if ("const" in definition) return value === definition.const;
  if (definition.type === "string") {
    if (typeof value !== "string") return false;
    switch (definition.format) {
      case "opaque": return value.length > 0 && encoder.encode(value).length <= definition.maxBytes!
        && !/[\u0000-\u001f\u007f-\u009f\ud800-\udfff]/u.test(value);
      case "sha256": return /^[a-f0-9]{64}$/.test(value);
      case "git-revision": return /^[a-f0-9]{40}$/.test(value);
      case "uint64":
      case "generation": return /^(0|[1-9][0-9]{0,19})$/.test(value)
        && BigInt(value) <= 18446744073709551615n
        && (definition.format !== "generation" || value !== "0");
      default: return false;
    }
  }
  if (definition.enum) return definition.enum.includes(value);
  if (definition.fields) {
    return object(value) && Object.keys(value).length === Object.keys(definition.fields).length
      && Object.entries(definition.fields).every(([key, field]) => Object.hasOwn(value, key) && matches(value[key], field));
  }
  if (definition.variants && definition.tag) {
    if (!object(value) || typeof value[definition.tag] !== "string") return false;
    const variant = definition.variants[value[definition.tag] as string];
    if (!variant) return false;
    const fields = { ...value };
    delete fields[definition.tag];
    return matches(fields, variant);
  }
  if (definition.items) {
    return Array.isArray(value) && value.length <= definition.maxItems!
      && value.every(item => matches(item, definition.items!))
      && (!definition.unique || new Set(value.map(canonical)).size === value.length);
  }
  return false;
}

function validate(value: unknown): asserts value is Message {
  if (!object(value) || value.schemaVersion !== schema.version) throw new ProtocolError("incompatible-version");
  if (!matches(value, schema.root)) throw new ProtocolError("invalid-frame");
}

/** Decode compact key-sorted JSON; transport must separately bound its reads. */
export function decode(bytes: Uint8Array): Message {
  if (bytes.length > schema.maxFrameBytes) throw new ProtocolError("invalid-frame");
  let value: unknown;
  let text: string;
  try {
    text = decoder.decode(bytes);
    value = JSON.parse(text);
    const encoded = encoder.encode(canonical(value));
    if (!canonicalNumbers(value) || encoded.length !== bytes.length
        || encoded.some((byte, index) => byte !== bytes[index])) throw new Error("noncanonical");
  } catch {
    throw new ProtocolError("invalid-frame");
  }
  validate(value);
  return value;
}

/** Validate before emitting the canonical transport document. */
export function encode(message: Message): Uint8Array {
  validate(message);
  const bytes = encoder.encode(canonical(message));
  if (bytes.length > schema.maxFrameBytes) throw new ProtocolError("invalid-frame");
  return bytes;
}

/** Discovery never substitutes for authenticated custodian observation. */
export function negotiate(request: Message, response: Message, required: Capability[]): void {
  encode(request);
  encode(response);
  if (request.body.kind !== "describe" || response.body.kind !== "capabilities") throw new ProtocolError("invalid-frame");
  if (request.requestId !== response.requestId) throw new ProtocolError("transport-failed");
  if (canonical(response.body.implementation) !== canonical(request.body.expectedImplementation)
      || request.body.expectedImplementation.schemaDigest !== schemaDigest) throw new ProtocolError("custody-lost");
  if (request.body.requiredProfile === "macos-untrusted-review" || !response.body.profiles.includes(request.body.requiredProfile)) throw new ProtocolError("unsupported-profile");
  const mandatory: Capability[] = ["authenticated-ipc", "durable-intents", "zero-effect-reopen", "descendant-cleanup"];
  const capabilities = response.body.capabilities;
  if ([...mandatory, ...required].some(capability => !capabilities.includes(capability))) throw new ProtocolError("unsupported-capability");
}
