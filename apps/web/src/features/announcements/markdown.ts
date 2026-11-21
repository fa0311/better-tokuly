/**
 * A small Markdown reader for announcement bodies: the everyday subset of
 * CommonMark (headings, paragraphs, emphasis, links, images, lists, quotes,
 * code, rules) parsed into a tree that `Markdown` renders as React elements.
 *
 * Raw HTML is never interpreted — it comes out as the text it is — and only
 * web, mail and in-app addresses become links or images, so a body can never
 * inject markup or a `javascript:` URL.
 *
 * One deliberate choice: a single line break inside a paragraph is kept as a
 * line break (CommonMark allows rendering soft breaks that way). Japanese has
 * no spaces between words, so joining lines with a space would be wrong.
 */

export type Inline =
  | { type: "text"; text: string }
  | { type: "strong" | "em" | "del"; children: Inline[] }
  | { type: "code"; text: string }
  | { type: "link"; href: string; children: Inline[] }
  | { type: "image"; src: string; alt: string }
  | { type: "break" };

export type Block =
  | { type: "heading"; level: number; children: Inline[] }
  | { type: "paragraph"; children: Inline[] }
  | { type: "code"; text: string; language: string }
  | { type: "quote"; children: Block[] }
  | { type: "list"; ordered: boolean; start: number; items: Block[][] }
  | { type: "rule" };

// ───────────────────────────── Blocks ─────────────────────────────

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const QUOTE = /^ {0,3}>[ ]?/;
const LIST_ITEM = /^( {0,3})([-*+]|(\d{1,9})[.)])(?:[ \t]+(.*))?$/;

const isBlank = (line: string) => line.trim() === "";

/** Whether `line` begins a block that ends the paragraph before it. */
function interruptsParagraph(line: string): boolean {
  if (FENCE.test(line) || HEADING.test(line) || RULE.test(line) || QUOTE.test(line)) return true;
  const item = LIST_ITEM.exec(line);
  // As in CommonMark: a numbered list only cuts into a paragraph when it starts at 1, and never an empty item.
  return Boolean(item && item[4] !== undefined && (item[3] === undefined || item[3] === "1"));
}

export function parseMarkdown(source: string): Block[] {
  return parseBlocks(source.replace(/\r\n?/g, "\n").split("\n"));
}

function parseBlocks(lines: string[]): Block[] {
  const blocks: Block[] = [];
  let index = 0;
  while (index < lines.length) {
    const line = lines[index] as string;
    if (isBlank(line)) {
      index++;
      continue;
    }

    const fence = FENCE.exec(line);
    if (fence) {
      const marker = fence[1] as string;
      const closing = new RegExp(`^ {0,3}${marker[0]}{${marker.length},}[ \\t]*$`);
      const body: string[] = [];
      index++;
      while (index < lines.length && !closing.test(lines[index] as string)) body.push(lines[index++] as string);
      index++; // the closing fence (or the end of the text)
      blocks.push({ type: "code", text: body.join("\n"), language: fence[2] ?? "" });
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading) {
      blocks.push({ type: "heading", level: (heading[1] as string).length, children: parseInline((heading[2] ?? "").trim()) });
      index++;
      continue;
    }

    if (RULE.test(line)) {
      blocks.push({ type: "rule" });
      index++;
      continue;
    }

    if (QUOTE.test(line)) {
      const quoted: string[] = [];
      while (index < lines.length && QUOTE.test(lines[index] as string)) quoted.push((lines[index++] as string).replace(QUOTE, ""));
      blocks.push({ type: "quote", children: parseBlocks(quoted) });
      continue;
    }

    const item = LIST_ITEM.exec(line);
    if (item) {
      const [list, next] = parseList(lines, index, item[3] !== undefined);
      blocks.push(list);
      index = next;
      continue;
    }

    const paragraph = [line];
    index++;
    while (index < lines.length && !isBlank(lines[index] as string) && !interruptsParagraph(lines[index] as string)) paragraph.push(lines[index++] as string);
    blocks.push({ type: "paragraph", children: parseInline(paragraph.map((text) => text.trim()).join("\n")) });
  }
  return blocks;
}

/** Reads the list starting at `start`; returns it with the index of the first line after it. */
function parseList(lines: string[], start: number, ordered: boolean): [Block, number] {
  const items: Block[][] = [];
  const first = LIST_ITEM.exec(lines[start] as string);
  let index = start;

  while (index < lines.length) {
    const marker = LIST_ITEM.exec(lines[index] as string);
    if (!marker || (marker[3] !== undefined) !== ordered || RULE.test(lines[index] as string)) break;
    // Text of the item lines up after "- " / "12. ": deeper lines belong to it.
    const indent = (marker[1] as string).length + (marker[2] as string).length + 1;
    const content = [marker[4] ?? ""];
    index++;
    while (index < lines.length) {
      const line = lines[index] as string;
      if (isBlank(line)) {
        // A blank line stays inside the item only when indented content follows.
        const following = lines.slice(index + 1).find((candidate) => !isBlank(candidate));
        if (following === undefined || leadingSpaces(following) < indent) break;
        content.push("");
        index++;
      } else if (leadingSpaces(line) >= indent) {
        content.push(line.slice(indent));
        index++;
      } else if (!LIST_ITEM.test(line) && !interruptsParagraph(line) && !isBlank(content[content.length - 1] as string)) {
        // A paragraph that simply runs on without indentation.
        content.push(line.trim());
        index++;
      } else {
        break;
      }
    }
    items.push(parseBlocks(content));
    // Blank lines between items do not end the list.
    let lookahead = index;
    while (lookahead < lines.length && isBlank(lines[lookahead] as string)) lookahead++;
    const nextMarker = lookahead < lines.length ? LIST_ITEM.exec(lines[lookahead] as string) : null;
    if (nextMarker && (nextMarker[3] !== undefined) === ordered && !RULE.test(lines[lookahead] as string)) index = lookahead;
    else break;
  }
  return [{ type: "list", ordered, start: ordered ? Number(first?.[3] ?? 1) : 1, items }, index];
}

function leadingSpaces(line: string): number {
  return line.length - line.trimStart().length;
}

// ───────────────────────────── Inlines ─────────────────────────────

const ESCAPABLE = /[!-/:-@[-`{-~]/;
const BARE_URL = /^https?:\/\/[!-;=?-~]+/;
const AUTOLINK = /^<((?:https?:\/\/|mailto:)[^\s<>]+)>/;
const WORD = /[\p{L}\p{N}]/u;

/** An address a link may point to: the web, mail, or a page of the app. Anything else is refused. */
export function safeHref(url: string): string | null {
  const trimmed = url.trim();
  if (/^(https?:\/\/|mailto:)/i.test(trimmed)) return trimmed;
  if (/^\/(?!\/)/.test(trimmed) || trimmed.startsWith("#")) return trimmed;
  return null;
}

/** An address an image may load from: the web or this origin. */
export function safeImageSrc(url: string): string | null {
  const trimmed = url.trim();
  return /^https?:\/\//i.test(trimmed) || /^\/(?!\/)/.test(trimmed) ? trimmed : null;
}

export function parseInline(text: string): Inline[] {
  const nodes: Inline[] = [];
  let buffer = "";
  const flush = () => {
    if (buffer) nodes.push({ type: "text", text: buffer });
    buffer = "";
  };
  const push = (node: Inline) => {
    flush();
    nodes.push(node);
  };

  let index = 0;
  while (index < text.length) {
    const char = text[index] as string;
    const rest = text.slice(index);

    if (char === "\\") {
      const next = text[index + 1];
      if (next === "\n") {
        push({ type: "break" });
        index += 2;
      } else if (next !== undefined && ESCAPABLE.test(next)) {
        buffer += next;
        index += 2;
      } else {
        buffer += char;
        index++;
      }
      continue;
    }

    if (char === "\n") {
      buffer = buffer.trimEnd();
      push({ type: "break" });
      index++;
      continue;
    }

    if (char === "`") {
      const ticks = (/^`+/.exec(rest) as RegExpExecArray)[0];
      const end = findCodeEnd(text, index + ticks.length, ticks.length);
      if (end >= 0) {
        let code = text.slice(index + ticks.length, end).replace(/\n/g, " ");
        if (code.length > 1 && code.startsWith(" ") && code.endsWith(" ") && code.trim() !== "") code = code.slice(1, -1);
        push({ type: "code", text: code });
        index = end + ticks.length;
      } else {
        buffer += ticks;
        index += ticks.length;
      }
      continue;
    }

    if (char === "!" && text[index + 1] === "[") {
      const link = readLink(text, index + 1);
      if (link) {
        const src = safeImageSrc(link.destination);
        // An image from somewhere we will not load from is reduced to its description.
        if (src) push({ type: "image", src, alt: plainText(parseInline(link.label)) });
        else buffer += plainText(parseInline(link.label));
        index = link.end;
        continue;
      }
    }

    if (char === "[") {
      const link = readLink(text, index);
      if (link) {
        const href = safeHref(link.destination);
        const children = parseInline(link.label);
        if (href) push({ type: "link", href, children });
        else for (const child of children) push(child);
        index = link.end;
        continue;
      }
    }

    if (char === "<") {
      const autolink = AUTOLINK.exec(rest);
      if (autolink) {
        const address = autolink[1] as string;
        push({ type: "link", href: address, children: [{ type: "text", text: address.replace(/^mailto:/, "") }] });
        index += autolink[0].length;
        continue;
      }
    }

    if (char === "h" && !WORD.test(text[index - 1] ?? "")) {
      const bare = BARE_URL.exec(rest);
      if (bare) {
        // Punctuation that ends the sentence is not part of the address.
        const address = bare[0].replace(/[.,;:!?'")\]]+$/, "");
        push({ type: "link", href: address, children: [{ type: "text", text: address }] });
        index += address.length;
        continue;
      }
    }

    if (char === "*" || char === "_" || char === "~") {
      const emphasis = readEmphasis(text, index);
      if (emphasis) {
        push(emphasis.node);
        index = emphasis.end;
        continue;
      }
    }

    buffer += char;
    index++;
  }
  flush();
  return nodes;
}

/** Index of the closing run of exactly `length` backticks, or -1. */
function findCodeEnd(text: string, from: number, length: number): number {
  let index = from;
  while (index < text.length) {
    if (text[index] !== "`") {
      index++;
      continue;
    }
    const run = (/^`+/.exec(text.slice(index)) as RegExpExecArray)[0].length;
    if (run === length) return index;
    index += run;
  }
  return -1;
}

/** Reads `[label](destination "title")` starting at the `[`. */
function readLink(text: string, start: number): { label: string; destination: string; end: number } | null {
  let depth = 0;
  let index = start;
  for (; index < text.length; index++) {
    const char = text[index];
    if (char === "\\") index++;
    else if (char === "`") {
      const ticks = (/^`+/.exec(text.slice(index)) as RegExpExecArray)[0].length;
      const end = findCodeEnd(text, index + ticks, ticks);
      if (end >= 0) index = end + ticks - 1;
    } else if (char === "[") depth++;
    else if (char === "]" && --depth === 0) break;
  }
  if (index >= text.length || text[index + 1] !== "(") return null;
  const target = /^\(\s*(?:<([^<>\n]*)>|((?:[^\s()\\]|\\.|\([^\s()]*\))*))(?:\s+(?:"[^"]*"|'[^']*'))?\s*\)/.exec(text.slice(index + 1));
  if (!target) return null;
  const destination = (target[1] ?? target[2] ?? "").replace(/\\(.)/g, "$1");
  return { label: text.slice(start + 1, index), destination, end: index + 1 + target[0].length };
}

/** Reads `**strong**`, `*em*`, `__strong__`, `_em_` or `~~del~~` starting at its opening marker. */
function readEmphasis(text: string, start: number): { node: Inline; end: number } | null {
  const char = text[start] as string;
  const double = text[start + 1] === char;
  if (char === "~" && !double) return null;
  const marker = double ? char + char : char;
  const after = text[start + marker.length];
  if (after === undefined || /\s/.test(after)) return null;
  // Underscores inside a word (snake_case) are just underscores.
  if (char === "_" && WORD.test(text[start - 1] ?? "")) return null;

  for (let index = start + marker.length; index < text.length; index++) {
    const current = text[index];
    if (current === "\\") {
      index++;
      continue;
    }
    if (current === "`") {
      const ticks = (/^`+/.exec(text.slice(index)) as RegExpExecArray)[0].length;
      const end = findCodeEnd(text, index + ticks, ticks);
      index = end >= 0 ? end + ticks - 1 : index + ticks - 1;
      continue;
    }
    if (current !== char) continue;
    // A run of markers: `**` closes with its last two, `*` with the last one of an odd run
    // (the others belong to a `**` inside); an even run cannot close a single marker.
    let run = 1;
    while (text[index + run] === char) run++;
    if (double ? run < 2 : run % 2 === 0) {
      index += run - 1;
      continue;
    }
    index += run - marker.length;
    if (/\s/.test(text[index - 1] as string) || index === start + marker.length) continue;
    if (char === "_" && WORD.test(text[index + marker.length] ?? "")) continue;
    const children = parseInline(text.slice(start + marker.length, index));
    const type = char === "~" ? "del" : double ? "strong" : "em";
    return { node: { type, children }, end: index + marker.length };
  }
  return null;
}

/** The text of inline nodes with all formatting dropped (image descriptions). */
export function plainText(nodes: Inline[]): string {
  return nodes
    .map((node) => {
      if (node.type === "text" || node.type === "code") return node.text;
      if (node.type === "image") return node.alt;
      if (node.type === "break") return " ";
      return plainText(node.children);
    })
    .join("");
}

/** The beginning of a Markdown text as plain prose, for previews of a summary that was left empty. */
export function markdownExcerpt(source: string, maxChars: number): string {
  const proseOf = (blocks: Block[]): string[] =>
    blocks.flatMap((block) => {
      if (block.type === "paragraph" || block.type === "heading") return [plainText(block.children).trim()];
      if (block.type === "quote") return proseOf(block.children);
      if (block.type === "list") return block.items.flatMap(proseOf);
      return [];
    });
  const prose = Array.from(proseOf(parseMarkdown(source)).filter(Boolean).join(" "));
  return prose.length > maxChars ? `${prose.slice(0, maxChars).join("")}…` : prose.join("");
}
