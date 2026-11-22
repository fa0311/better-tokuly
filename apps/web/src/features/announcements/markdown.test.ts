import { describe, expect, it } from "vitest";
import {
  type Block,
  type Inline,
  parseInline,
  parseMarkdown,
  safeHref,
  safeImageSrc,
} from "./markdown";

const text = (value: string): Inline => ({ type: "text", text: value });
const paragraph = (...children: Inline[]): Block => ({ type: "paragraph", children });

describe("markdown blocks", () => {
  it("reads headings, paragraphs and rules", () => {
    expect(parseMarkdown("# 新機能\n\nマップが速くなりました!\n\n---\n\n### 詳細 ###")).toEqual([
      { type: "heading", level: 1, children: [text("新機能")] },
      paragraph(text("マップが速くなりました!")),
      { type: "rule" },
      { type: "heading", level: 3, children: [text("詳細")] },
    ]);
  });

  it("keeps a single line break inside a paragraph as a break", () => {
    expect(parseMarkdown("一行目\n二行目")).toEqual([
      paragraph(text("一行目"), { type: "break" }, text("二行目")),
    ]);
  });

  it("reads bullet and numbered lists, with nesting by indentation", () => {
    expect(parseMarkdown("- りんご\n- みかん\n  - 小さいの\n\n3. 三\n4. 四")).toEqual([
      {
        type: "list",
        ordered: false,
        start: 1,
        items: [
          [paragraph(text("りんご"))],
          [
            paragraph(text("みかん")),
            { type: "list", ordered: false, start: 1, items: [[paragraph(text("小さいの"))]] },
          ],
        ],
      },
      {
        type: "list",
        ordered: true,
        start: 3,
        items: [[paragraph(text("三"))], [paragraph(text("四"))]],
      },
    ]);
  });

  it("lets a list follow a paragraph without a blank line, but not a number other than 1", () => {
    expect(parseMarkdown("買うもの:\n- 牛乳")).toEqual([
      paragraph(text("買うもの:")),
      { type: "list", ordered: false, start: 1, items: [[paragraph(text("牛乳"))]] },
    ]);
    expect(parseMarkdown("バージョン\n2. 続き")).toEqual([
      paragraph(text("バージョン"), { type: "break" }, text("2. 続き")),
    ]);
  });

  it("reads fenced code verbatim and quotes recursively", () => {
    expect(parseMarkdown("```ts\nconst a = **1**;\n<b>x</b>\n```\n\n> 引用\n> - 項目")).toEqual([
      { type: "code", language: "ts", text: "const a = **1**;\n<b>x</b>" },
      {
        type: "quote",
        children: [
          paragraph(text("引用")),
          { type: "list", ordered: false, start: 1, items: [[paragraph(text("項目"))]] },
        ],
      },
    ]);
  });

  it("treats an unclosed fence as code to the end, and blank input as nothing", () => {
    expect(parseMarkdown("~~~\nabc")).toEqual([{ type: "code", language: "", text: "abc" }]);
    expect(parseMarkdown("\n \n")).toEqual([]);
  });
});

describe("markdown inlines", () => {
  it("reads emphasis, also without spaces around it as in Japanese", () => {
    expect(parseInline("これは**重要**で*すこし*~~古い~~")).toEqual([
      text("これは"),
      { type: "strong", children: [text("重要")] },
      text("で"),
      { type: "em", children: [text("すこし")] },
      { type: "del", children: [text("古い")] },
    ]);
    expect(parseInline("***both***")).toEqual([
      { type: "strong", children: [{ type: "em", children: [text("both")] }] },
    ]);
  });

  it("leaves lone markers and snake_case alone", () => {
    expect(parseInline("2 * 3 * 4")).toEqual([text("2 * 3 * 4")]);
    expect(parseInline("new_map_pins と **")).toEqual([text("new_map_pins と **")]);
  });

  it("reads code spans without interpreting what is inside", () => {
    expect(parseInline("`**x**` と `` a`b ``")).toEqual([
      { type: "code", text: "**x**" },
      text(" と "),
      { type: "code", text: "a`b" },
    ]);
  });

  it("reads links, autolinks and bare addresses", () => {
    expect(parseInline('[ヘルプ](https://tokuly.example/help "title")を見てね')).toEqual([
      { type: "link", href: "https://tokuly.example/help", children: [text("ヘルプ")] },
      text("を見てね"),
    ]);
    expect(parseInline("詳しくは https://tokuly.example/a?b=1. です")).toEqual([
      text("詳しくは "),
      {
        type: "link",
        href: "https://tokuly.example/a?b=1",
        children: [text("https://tokuly.example/a?b=1")],
      },
      text(". です"),
    ]);
    expect(parseInline("<mailto:hi@tokuly.example>")).toEqual([
      { type: "link", href: "mailto:hi@tokuly.example", children: [text("hi@tokuly.example")] },
    ]);
    expect(parseInline("[設定](/settings)")).toEqual([
      { type: "link", href: "/settings", children: [text("設定")] },
    ]);
  });

  it("never produces a link or image for an unsafe address", () => {
    expect(parseInline("[押して](javascript:alert(1))")).toEqual([text("押して")]);
    expect(parseInline("![罠](data:image/svg+xml,x)")).toEqual([text("罠")]);
    expect(safeHref("JavaScript:alert(1)")).toBeNull();
    expect(safeHref("//evil.example")).toBeNull();
    expect(safeImageSrc("//evil.example/a.png")).toBeNull();
    expect(safeImageSrc("https://tokuly.example/a.png")).toBe("https://tokuly.example/a.png");
  });

  it("keeps raw HTML as text", () => {
    expect(parseInline('<script>alert("x")</script> <b>太字</b>')).toEqual([
      text('<script>alert("x")</script> <b>太字</b>'),
    ]);
  });

  it("reads images and escapes", () => {
    expect(parseInline("![**地図**の画面](/media/a.webp)")).toEqual([
      { type: "image", src: "/media/a.webp", alt: "地図の画面" },
    ]);
    expect(parseInline("\\*そのまま\\* \\[x\\]")).toEqual([text("*そのまま* [x]")]);
  });
});
