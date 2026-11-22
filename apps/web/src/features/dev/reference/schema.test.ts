import { describe, expect, it } from "vitest";
import {
  exampleOf,
  fieldsOf,
  nestedRef,
  parseSchema,
  reasonsOf,
  resolve,
  type SchemaDocument,
  typeLabel,
} from "./schema";

// The shape the server writes: one definition per message and enum, the root a reference.
const DOCUMENT: SchemaDocument = {
  $ref: "#/$defs/t.v1.CreateRequest",
  $defs: {
    "t.v1.CreateRequest": {
      type: "object",
      properties: {
        name: { type: "string", description: "1-60 characters." },
        scopes: { type: "array", items: { type: "string" } },
        expiry: { $ref: "#/$defs/t.v1.Expiry" },
        count: { type: "integer", format: "int32" },
        total: { type: ["string", "integer"], format: "int64" },
        at: { type: "string", format: "date-time" },
        flags: { type: "object", additionalProperties: { type: "boolean" } },
        parent: { $ref: "#/$defs/t.v1.Node" },
        tokenId: { type: "string" },
        appId: { type: "string" },
      },
      "x-oneofs": { subject: ["tokenId", "appId"] },
    },
    "t.v1.Expiry": { type: "string", enum: ["EXPIRY_UNSPECIFIED", "EXPIRY_7_DAYS"] },
    "t.v1.Node": {
      type: "object",
      properties: {
        id: { type: "string" },
        children: { type: "array", items: { $ref: "#/$defs/t.v1.Node" } },
      },
    },
  },
};

describe("schema", () => {
  it("parses a schema string and refuses what is not one", () => {
    expect(parseSchema(JSON.stringify(DOCUMENT))?.$ref).toBe("#/$defs/t.v1.CreateRequest");
    expect(parseSchema("")).toBeUndefined();
    expect(parseSchema("{nope")).toBeUndefined();
  });

  it("names field types the way the proto does", () => {
    const root = resolve(DOCUMENT, DOCUMENT);
    const types = Object.fromEntries(
      fieldsOf(root).map((field) => [field.name, typeLabel(field.node)]),
    );
    expect(types).toEqual({
      name: "string",
      scopes: "string[]",
      expiry: "Expiry",
      count: "int32",
      total: "int64",
      at: "timestamp",
      flags: "map<string, bool>",
      parent: "Node",
      tokenId: "string",
      appId: "string",
    });
  });

  it("knows which fields exclude each other and what a field opens", () => {
    const fields = fieldsOf(resolve(DOCUMENT, DOCUMENT));
    expect(fields.find((field) => field.name === "appId")?.oneof).toBe("subject");
    expect(fields.find((field) => field.name === "name")?.oneof).toBeUndefined();
    const node = DOCUMENT.$defs?.["t.v1.Node"]?.properties?.children;
    expect(node && nestedRef(node)).toBe("t.v1.Node");
  });

  it("builds an example with every field, one oneof alternative and no endless nesting", () => {
    expect(exampleOf(DOCUMENT)).toEqual({
      name: "",
      scopes: [""],
      expiry: "EXPIRY_7_DAYS",
      count: 0,
      total: "0",
      at: "2026-01-01T00:00:00Z",
      flags: {},
      parent: { id: "", children: [] },
      tokenId: "",
    });
  });
});

describe("reasonsOf", () => {
  it("lists the reasons a description names, once each", () => {
    expect(
      reasonsOf(
        "Creates a token (`tkl_pat_…`). Reasons: `TOKEN_NAME_INVALID`,\n`SCOPES_REQUIRED` (none given), `TOKEN_NAME_INVALID`.",
      ),
    ).toEqual(["TOKEN_NAME_INVALID", "SCOPES_REQUIRED"]);
    expect(reasonsOf("Errors: PURPOSE_INVALID, FILE_TOO_LARGE.")).toEqual([
      "PURPOSE_INVALID",
      "FILE_TOO_LARGE",
    ]);
  });

  it("finds none where the description names none", () => {
    expect(reasonsOf("Returns `ASSET_PURPOSE_AVATAR` images.")).toEqual([]);
    expect(reasonsOf("")).toEqual([]);
  });
});
