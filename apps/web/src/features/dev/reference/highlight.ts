/**
 * Syntax highlighting for the three kinds of code the reference shows: JSON,
 * shell (`curl`) and TypeScript. A small lexer per language instead of a
 * highlighting library: the page stays light and the result is exact for the
 * code we generate ourselves.
 */

export type Language = "json" | "shell" | "ts" | "text";

export type TokenType =
  | "plain"
  | "comment"
  | "string"
  | "number"
  | "keyword"
  | "property"
  | "variable"
  | "flag"
  | "function"
  | "punctuation";

export interface Token {
  type: TokenType;
  text: string;
}

type Rule = [type: TokenType, pattern: RegExp];

const DOUBLE = String.raw`"(?:[^"\\\n]|\\.)*"`;
const SINGLE = String.raw`'(?:[^'\\\n]|\\.)*'`;
const NUMBER = String.raw`-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b`;

const TS_KEYWORDS = [
  "as",
  "async",
  "await",
  "break",
  "catch",
  "class",
  "const",
  "continue",
  "else",
  "export",
  "false",
  "for",
  "from",
  "function",
  "if",
  "import",
  "in",
  "interface",
  "let",
  "new",
  "null",
  "of",
  "return",
  "throw",
  "true",
  "try",
  "type",
  "typeof",
  "undefined",
  "while",
];

// Sticky patterns, tried in order at the current position.
const rule = (type: TokenType, source: string): Rule => [type, new RegExp(source, "y")];

const RULES: Record<Language, Rule[]> = {
  text: [],
  json: [
    rule("property", `${DOUBLE}(?=\\s*:)`),
    rule("string", DOUBLE),
    rule("number", NUMBER),
    rule("keyword", String.raw`\b(?:true|false|null)\b`),
    rule("punctuation", String.raw`[{}\[\],:]`),
  ],
  shell: [
    rule("comment", String.raw`#[^\n]*`),
    rule("string", `${DOUBLE}|${SINGLE}`),
    rule("variable", String.raw`\$(?:\w+|\{\w+\})`),
    rule("flag", String.raw`(?<=^|\s)--?[A-Za-z][\w-]*`),
    rule("keyword", String.raw`(?<=^|\n)\s*(?:curl|export)\b`),
    rule("punctuation", String.raw`\\(?=\n|$)`),
  ],
  ts: [
    rule("comment", String.raw`//[^\n]*|/\*[\s\S]*?\*/`),
    rule("string", `${DOUBLE}|${SINGLE}|\`(?:[^\`\\\\]|\\\\.)*\``),
    rule("number", NUMBER),
    rule("keyword", `\\b(?:${TS_KEYWORDS.join("|")})\\b`),
    rule("function", String.raw`\b[A-Za-z_$][\w$]*(?=\()`),
    rule("property", String.raw`\b[A-Za-z_$][\w$]*(?=\s*:)`),
    rule("punctuation", String.raw`[{}()\[\];,.:]`),
  ],
};

/** `code` as tokens; joined back together they are exactly `code`. */
export function highlight(code: string, language: Language): Token[] {
  const rules = RULES[language];
  const tokens: Token[] = [];
  let plain = "";
  const flush = () => {
    if (plain) tokens.push({ type: "plain", text: plain });
    plain = "";
  };
  let position = 0;
  scan: while (position < code.length) {
    for (const [type, pattern] of rules) {
      pattern.lastIndex = position;
      const match = pattern.exec(code);
      if (match?.[0]) {
        flush();
        tokens.push({ type, text: match[0] });
        position += match[0].length;
        continue scan;
      }
    }
    // Words and runs of space go in one piece, so a rule never starts inside a word.
    const run = /[A-Za-z_$][\w$]*|\s+|[\s\S]/y;
    run.lastIndex = position;
    const text = run.exec(code)?.[0] ?? code[position] ?? "";
    plain += text;
    position += text.length;
  }
  flush();
  return tokens;
}
