// Writes crates/t3-contracts/schema/contracts.json: the JSON Schema (wire encoding) of every
// WebSocket RPC's payload, success, stream item and error, from packages/contracts. The Rust
// types in t3-contracts are generated from it. Run from the repo root after `vp i`:
//   node apps/rune/scripts/export-contract-schemas.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(import.meta.dirname, "../crates/t3-contracts/schema/contracts.json");
const serverUrl = NodeURL.pathToFileURL(
  NodePath.resolve(repoRoot, "apps/server/package.json"),
).href;
const scriptsUrl = NodeURL.pathToFileURL(import.meta.dirname).href;

// apps/rune has no node_modules; resolve packages as apps/server does.
NodeModule.registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if (parent.startsWith(scriptsUrl) && !specifier.startsWith(".") && !specifier.includes(":")) {
      return next(specifier, { ...context, parentURL: serverUrl });
    }
    return next(specifier, context);
  },
});

const { WsRpcGroup } = await import("@t3tools/contracts");
const Schema = await import("effect/Schema");

type JsonSchema = Record<string, unknown>;
type Rpc = {
  readonly _tag: string;
  readonly payloadSchema: Schema.Top;
  readonly successSchema: Schema.Top & {
    readonly success?: Schema.Top;
    readonly error?: Schema.Top;
  };
  readonly errorSchema: Schema.Top;
};

const STREAM = "~effect/rpc/RpcSchema/StreamSchema";

/** `orchestration.dispatchCommand` → `OrchestrationDispatchCommand`. */
function typeName(tag: string): string {
  return tag
    .split(/[.\-_]/)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
}

const defs: Record<string, JsonSchema> = {};
const rpcs: Array<{ tag: string; name: string; stream: boolean }> = [];

/** Rewrites `#/$defs/X` references after a definition is renamed on collision. */
function renameRefs(node: unknown, renames: Map<string, string>): unknown {
  if (Array.isArray(node)) return node.map((n) => renameRefs(n, renames));
  if (node === null || typeof node !== "object") return node;
  return Object.fromEntries(
    Object.entries(node).map(([k, v]) => {
      if (k === "$ref" && typeof v === "string" && v.startsWith("#/$defs/")) {
        const target = v.slice("#/$defs/".length);
        return [k, `#/$defs/${renames.get(target) ?? target}`];
      }
      return [k, renameRefs(v, renames)];
    }),
  );
}

type Doc = { schema: JsonSchema; definitions?: Record<string, JsonSchema> };

function add(name: string, schema: Schema.Top) {
  let doc = Schema.toJsonSchemaDocument(schema) as Doc;
  // A codec that decodes from raw JSON (`withRecordIdTable`) has an opaque encoded side; its wire
  // shape is the JSON codec of its type side.
  if (Object.keys(doc.schema).length === 0) {
    doc = Schema.toJsonSchemaDocument(Schema.toCodecJson(Schema.toType(schema))) as Doc;
  }
  // Shared definitions (ids, commands, turn items) recur across RPCs; identical ones merge, and a
  // differing one with the same identifier is kept under a suffixed name.
  const renames = new Map<string, string>();
  for (const [id, def] of Object.entries(doc.definitions ?? {})) {
    const existing = defs[id];
    if (existing === undefined || JSON.stringify(existing) === JSON.stringify(def)) continue;
    let n = 2;
    while (
      defs[`${id}${n}`] !== undefined &&
      JSON.stringify(defs[`${id}${n}`]) !== JSON.stringify(def)
    )
      n += 1;
    renames.set(id, `${id}${n}`);
  }
  for (const [id, def] of Object.entries(doc.definitions ?? {})) {
    defs[renames.get(id) ?? id] = renameRefs(def, renames) as JsonSchema;
  }
  defs[name] = renameRefs(doc.schema, renames) as JsonSchema;
}

const requests = [
  ...(WsRpcGroup as unknown as { requests: Map<string, Rpc> }).requests.values(),
].toSorted((a, b) => a._tag.localeCompare(b._tag));
for (const rpc of requests) {
  const name = typeName(rpc._tag);
  const stream = STREAM in rpc.successSchema;
  add(`${name}Payload`, rpc.payloadSchema);
  if (stream) {
    add(`${name}Item`, rpc.successSchema.success as Schema.Top);
  } else {
    add(`${name}Success`, rpc.successSchema);
  }
  add(`${name}Error`, rpc.errorSchema);
  rpcs.push({ tag: rpc._tag, name, stream });
}

// Effect inlines a schema at every use, so a generator would emit one type per occurrence and
// invent clashing names. Hoist every object and union into a named definition: the name the
// contracts module exports it under, else one from the property (or union tag) it sits under.
const canonical = (node: unknown): string =>
  JSON.stringify(node, (_k, v) =>
    v !== null && typeof v === "object" && !Array.isArray(v)
      ? Object.fromEntries(Object.entries(v).toSorted(([a], [b]) => a.localeCompare(b)))
      : v,
  );
const isNull = (node: unknown) => (node as JsonSchema | null)?.type === "null";
// A nullable wrapper (`anyOf: [X, null]`) stays inline so generators emit `Option<X>`; X itself
// is what gets a name.
const hoistable = (node: JsonSchema): boolean =>
  (node.type === "object" &&
    Object.keys((node.properties as object | undefined) ?? {}).length >= 2) ||
  (Array.isArray(node.anyOf) && node.anyOf.filter((m) => !isNull(m)).length >= 2);

const exported = new Map<string, string>();
const contracts = await import("@t3tools/contracts");
for (const [name, value] of Object.entries(contracts).toSorted(([a], [b]) => a.localeCompare(b))) {
  if (!Schema.isSchema(value)) continue;
  try {
    const doc = Schema.toJsonSchemaDocument(value as Schema.Top) as { schema: JsonSchema };
    const key = canonical(doc.schema);
    if (hoistable(doc.schema) && !exported.has(key)) exported.set(key, name);
  } catch {
    // Declarations with no JSON form (streams, classes) have nothing to match.
  }
}

const pascal = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/[^A-Za-z0-9]/g, "");

const assigned = new Map<string, string>();
const nameFor = (key: string, hint: string): string | undefined => {
  const known = exported.get(key) ?? assigned.get(key);
  if (known !== undefined) return known;
  // Every object and union gets a collision-checked name here, so generators never invent two
  // types with one name from the same property.
  let name = pascal(hint) || "Shared";
  for (let n = 2; defs[name] !== undefined || [...assigned.values()].includes(name); n += 1)
    name = `${pascal(hint)}${n}`;
  assigned.set(key, name);
  return name;
};
const hoist = (node: unknown, hint: string, root: boolean): unknown => {
  if (Array.isArray(node)) return node.map((n) => hoist(n, hint, false));
  if (node === null || typeof node !== "object") return node;
  if (!root && hoistable(node as JsonSchema)) {
    const key = canonical(node);
    const name = nameFor(key, hint);
    if (name !== undefined) {
      if (defs[name] === undefined) defs[name] = {}; // reserve before recursing (recursive types)
      if (Object.keys(defs[name]).length === 0) defs[name] = hoist(node, name, true) as JsonSchema;
      return { $ref: `#/$defs/${name}` };
    }
  }
  return Object.fromEntries(
    Object.entries(node).map(([k, v]) => {
      if (k === "anyOf" && Array.isArray(v))
        return [k, v.map((m) => hoist(m, variantHint(hint, m), false))];
      if (k === "items") return [k, hoist(v, singular(hint), false)];
      return [k, hoist(v, k === "properties" ? hint : k, false)];
    }),
  );
};

const singular = (hint: string) =>
  hint.endsWith("ies") ? `${hint.slice(0, -3)}y` : hint.replace(/s$/, "");
/** A tagged union member is named by its tag: `TurnItem` + `user_message` → `TurnItemUserMessage`. */
function variantHint(hint: string, member: unknown): string {
  const props = (member as JsonSchema | null)?.properties as Record<string, JsonSchema> | undefined;
  for (const tag of ["type", "kind", "_tag"]) {
    const values = props?.[tag]?.enum;
    if (Array.isArray(values) && values.length === 1 && typeof values[0] === "string") {
      return `${hint}${pascal(values[0].replace(/[._-](\w)/g, (_m, c: string) => c.toUpperCase()))}`;
    }
  }
  return hint;
}
for (const [name, def] of Object.entries(defs)) defs[name] = hoist(def, name, true) as JsonSchema;
// A definition that is itself `anyOf: [inline X, null]` would give generators two types with one
// name; X moves to `<Name>Value`.
for (const [name, def] of Object.entries(defs)) {
  const members = def.anyOf;
  if (!Array.isArray(members) || members.length !== 2 || !members.some(isNull)) continue;
  const inner = members.find((m) => !isNull(m)) as JsonSchema;
  if (inner.$ref !== undefined || !hoistable(inner)) continue;
  defs[`${name}Value`] = inner;
  defs[name] = {
    ...def,
    anyOf: members.map((m) => (m === inner ? { $ref: `#/$defs/${name}Value` } : m)),
  };
}

const document = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  title: "T3 Code WebSocket RPC contracts (wire encoding)",
  "x-rpcs": rpcs,
  $defs: Object.fromEntries(Object.entries(defs).toSorted(([a], [b]) => a.localeCompare(b))),
};
NodeFS.mkdirSync(NodePath.dirname(out), { recursive: true });
NodeFS.writeFileSync(out, `${JSON.stringify(document, null, 1)}\n`);
console.log(`wrote ${out}: ${rpcs.length} rpcs, ${Object.keys(defs).length} definitions`);
