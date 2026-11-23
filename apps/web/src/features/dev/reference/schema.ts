/**
 * Reading the JSON Schemas the reference service returns for a method's
 * request and response (`ApiMethod.request_schema`): one document per
 * message, with every message and enum it uses under `$defs`.
 */

/** A node of such a schema: only the keywords the server writes. */
export interface SchemaNode {
  $ref?: string;
  type?: string | string[];
  format?: string;
  description?: string;
  properties?: Record<string, SchemaNode>;
  items?: SchemaNode;
  additionalProperties?: SchemaNode | boolean;
  enum?: string[];
  contentEncoding?: string;
  /** Notes on enum values, by value. */
  "x-enum-descriptions"?: Record<string, string>;
  /** Fields that exclude each other, by the name of their oneof. */
  "x-oneofs"?: Record<string, string[]>;
}

export interface SchemaDocument extends SchemaNode {
  $defs?: Record<string, SchemaNode>;
}

/** The document of a schema string; `undefined` when it is empty or not JSON. */
export function parseSchema(text: string): SchemaDocument | undefined {
  if (!text) return undefined;
  try {
    const value: unknown = JSON.parse(text);
    return value && typeof value === "object" ? (value as SchemaDocument) : undefined;
  } catch {
    return undefined;
  }
}

const REF_PREFIX = "#/$defs/";

/** The full name of the definition a node points at (`tokuly.notes.v1.Note`). */
export function refName(node: SchemaNode): string | undefined {
  return node.$ref?.startsWith(REF_PREFIX) ? node.$ref.slice(REF_PREFIX.length) : undefined;
}

/** The definition a node points at, or the node itself when it is inline. */
export function resolve(document: SchemaDocument, node: SchemaNode): SchemaNode {
  const name = refName(node);
  return (name && document.$defs?.[name]) || node;
}

/** `tokuly.notes.v1.Note` → `Note`. */
export function shortName(fullName: string): string {
  return fullName.slice(fullName.lastIndexOf(".") + 1);
}

/** The definition a field leads to, through a list or a map: what can be opened below it. */
export function nestedRef(node: SchemaNode): string | undefined {
  if (node.items) return nestedRef(node.items);
  if (typeof node.additionalProperties === "object") return nestedRef(node.additionalProperties);
  return refName(node);
}

/** A field's type as an engineer would write it: `string`, `int64`, `Note[]`, `map<string, bool>`. */
export function typeLabel(node: SchemaNode): string {
  const name = refName(node);
  if (name) return shortName(name);
  if (node.items) return `${typeLabel(node.items)}[]`;
  if (typeof node.additionalProperties === "object")
    return `map<string, ${typeLabel(node.additionalProperties)}>`;
  if (node.format === "date-time") return "timestamp";
  if (node.contentEncoding === "base64") return "bytes";
  // 64-bit integers travel as strings in JSON; the format names the real type.
  if (Array.isArray(node.type)) return node.format ?? node.type.join(" | ");
  if (node.type === "integer" || node.type === "number") return node.format ?? node.type;
  if (node.type === "boolean") return "bool";
  return node.type ?? "any";
}

export interface SchemaField {
  name: string;
  node: SchemaNode;
  /** The oneof the field belongs to: at most one field of it may be set. */
  oneof?: string;
}

/** The fields of a message definition, in the server's order. */
export function fieldsOf(definition: SchemaNode): SchemaField[] {
  const oneofs = definition["x-oneofs"] ?? {};
  return Object.entries(definition.properties ?? {}).map(([name, node]) => ({
    name,
    node,
    oneof: Object.keys(oneofs).find((group) => oneofs[group]?.includes(name)),
  }));
}

/** How deep an example follows nested messages before it stops with `{}`. */
const EXAMPLE_DEPTH = 3;

/**
 * A value of the right shape to start editing from: every field present with
 * an empty value, the first real value of each enum, one alternative per
 * oneof, one element per list.
 */
export function exampleOf(document: SchemaDocument, node: SchemaNode = document): unknown {
  return example(document, node, []);
}

function example(document: SchemaDocument, node: SchemaNode, path: string[]): unknown {
  const name = refName(node);
  if (name) {
    // A message inside itself (a reply inside a comment) or too deep: stop.
    if (path.includes(name) || path.length >= EXAMPLE_DEPTH) {
      return document.$defs?.[name]?.enum ? enumExample(document.$defs[name]) : {};
    }
    return example(document, document.$defs?.[name] ?? {}, [...path, name]);
  }
  if (node.enum) return enumExample(node);
  if (node.items) {
    const item = example(document, node.items, path);
    return isEmptyObject(item) ? [] : [item];
  }
  if (node.properties) {
    const skipped = new Set(
      Object.values(node["x-oneofs"] ?? {}).flatMap((alternatives) => alternatives.slice(1)),
    );
    return Object.fromEntries(
      Object.entries(node.properties)
        .filter(([field]) => !skipped.has(field))
        .map(([field, child]) => [field, example(document, child, path)]),
    );
  }
  if (node.format === "date-time") return "2026-01-01T00:00:00Z";
  if (Array.isArray(node.type)) return "0";
  switch (node.type) {
    case "integer":
    case "number":
      return 0;
    case "boolean":
      return false;
    case "object":
      return {};
    case "array":
      return [];
    default:
      return "";
  }
}

/** The first value that means something: `*_UNSPECIFIED` is what leaving the field out means. */
function enumExample(node: SchemaNode): string {
  return node.enum?.find((value) => !value.endsWith("_UNSPECIFIED")) ?? node.enum?.[0] ?? "";
}

function isEmptyObject(value: unknown): boolean {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.keys(value).length === 0
  );
}

/**
 * The error reasons a method's documentation names: the `UPPER_SNAKE` words
 * after "Reasons:" (or "Errors:"), in the order they are written.
 */
export function reasonsOf(description: string): string[] {
  const start = description.search(/\b(?:Reasons?|Errors?)\b[^:.]*:/);
  if (start < 0) return [];
  const names = description.slice(start).match(/\b[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+\b/g) ?? [];
  return [...new Set(names)];
}
