import { describe, expect, it } from "vitest";
import { highlight, type Language } from "./highlight";

const typed = (code: string, language: Language) =>
  highlight(code, language)
    .filter((token) => token.type !== "plain")
    .map((token) => `${token.type}:${token.text}`);

describe("highlight", () => {
  it("never loses or changes a character", () => {
    const samples: [string, Language][] = [
      ['{ "a": [1, -2.5e3, true, null], "b\\"c": "d" }', "json"],
      ["curl https://x/api \\\n  -H 'a: b' -d \"$TOKEN\" # done", "shell"],
      // biome-ignore lint/suspicious/noTemplateCurlyInString: a sample of code, as text.
      ["const a = await fetch(`x${y}`, { method: 'POST' }); // go", "ts"],
      ["anything at all: {}", "text"],
    ];
    for (const [code, language] of samples)
      expect(
        highlight(code, language)
          .map((token) => token.text)
          .join(""),
      ).toBe(code);
  });

  it("tells JSON keys from string values", () => {
    expect(typed('{"name": "x", "n": 1, "ok": false}', "json")).toEqual([
      "punctuation:{",
      'property:"name"',
      "punctuation::",
      'string:"x"',
      "punctuation:,",
      'property:"n"',
      "punctuation::",
      "number:1",
      "punctuation:,",
      'property:"ok"',
      "punctuation::",
      "keyword:false",
      "punctuation:}",
    ]);
  });

  it("marks the command, flags, variables and strings of a curl call", () => {
    expect(
      typed("curl https://x \\\n  -H \"Authorization: Bearer $T\" --data '{}'", "shell"),
    ).toEqual([
      "keyword:curl",
      "punctuation:\\",
      "flag:-H",
      'string:"Authorization: Bearer $T"',
      "flag:--data",
      "string:'{}'",
    ]);
  });

  it("does not take a dash inside a word or a URL for a flag", () => {
    expect(typed("curl https://a-b.example/x-y", "shell")).toEqual(["keyword:curl"]);
  });

  it("marks keywords, calls, strings and comments in TypeScript", () => {
    const tokens = typed('const r = await fetch("u"); // c', "ts");
    expect(tokens).toContain("keyword:const");
    expect(tokens).toContain("keyword:await");
    expect(tokens).toContain("function:fetch");
    expect(tokens).toContain('string:"u"');
    expect(tokens).toContain("comment:// c");
  });

  it("does not find keywords inside identifiers", () => {
    expect(typed("format = information", "ts")).toEqual([]);
  });
});
