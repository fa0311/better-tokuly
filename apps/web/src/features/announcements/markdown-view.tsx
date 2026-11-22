import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";
import { type Block, type Inline, parseMarkdown } from "./markdown";

/**
 * Renders a Markdown text (see `markdown.ts` for what is understood). Nothing
 * in the text can become markup of its own: the tree is turned into React
 * elements, never into an HTML string.
 */
export function Markdown({ source, className }: { source: string; className?: string }) {
  return (
    <div
      className={cn("min-w-0 break-words [&>*:first-child]:mt-0 [&>*:last-child]:mb-0", className)}
    >
      {renderBlocks(parseMarkdown(source))}
    </div>
  );
}

function renderBlocks(blocks: Block[]): ReactNode {
  return blocks.map((block, index) => <BlockView key={index} block={block} />);
}

function BlockView({ block }: { block: Block }) {
  switch (block.type) {
    case "heading": {
      // The page title is the h1; headings of a body start one level below it.
      if (block.level === 1)
        return (
          <h2 className="mt-7 mb-2 text-xl leading-snug font-black">
            {renderInline(block.children)}
          </h2>
        );
      if (block.level === 2)
        return (
          <h3 className="mt-6 mb-2 text-lg leading-snug font-black">
            {renderInline(block.children)}
          </h3>
        );
      return <h4 className="mt-5 mb-1.5 font-black">{renderInline(block.children)}</h4>;
    }
    case "paragraph":
      return <p className="my-3 leading-relaxed">{renderInline(block.children)}</p>;
    case "code":
      return (
        <pre className="my-3 overflow-x-auto rounded-2xl bg-surface-2 p-4 text-sm leading-relaxed">
          <code className="font-mono">{block.text}</code>
        </pre>
      );
    case "quote":
      return (
        <blockquote className="my-3 border-l-4 border-brand pl-4 text-ink-soft [&>*:first-child]:mt-0 [&>*:last-child]:mb-0">
          {renderBlocks(block.children)}
        </blockquote>
      );
    case "list": {
      const items = block.items.map((item, index) => (
        <li key={index} className="pl-1 leading-relaxed [&>ol]:my-1 [&>ul]:my-1">
          {item.map((child, childIndex) =>
            // A list item's own text sits on the bullet's line, not in a spaced paragraph.
            child.type === "paragraph" ? (
              <div key={childIndex}>{renderInline(child.children)}</div>
            ) : (
              <BlockView key={childIndex} block={child} />
            ),
          )}
        </li>
      ));
      return block.ordered ? (
        <ol
          start={block.start}
          className="my-3 flex list-decimal flex-col gap-1 pl-6 marker:font-bold marker:text-ink-soft"
        >
          {items}
        </ol>
      ) : (
        <ul className="my-3 flex list-disc flex-col gap-1 pl-6 marker:text-brand">{items}</ul>
      );
    }
    case "rule":
      return <hr className="my-6 border-0 border-t-2 border-line" />;
  }
}

function renderInline(nodes: Inline[]): ReactNode {
  return nodes.map((node, index) => <InlineView key={index} node={node} />);
}

function InlineView({ node }: { node: Inline }) {
  switch (node.type) {
    case "text":
      return node.text;
    case "strong":
      return <strong className="font-black">{renderInline(node.children)}</strong>;
    case "em":
      return <em>{renderInline(node.children)}</em>;
    case "del":
      return <del className="text-ink-soft">{renderInline(node.children)}</del>;
    case "code":
      return (
        <code className="rounded-md bg-surface-2 px-1.5 py-0.5 font-mono text-[0.9em]">
          {node.text}
        </code>
      );
    case "break":
      return <br />;
    case "image":
      return (
        <img
          src={node.src}
          alt={node.alt}
          loading="lazy"
          decoding="async"
          className="my-3 block h-auto max-w-full rounded-2xl border-2 border-line"
        />
      );
    case "link": {
      const classes =
        "font-bold text-brand underline decoration-2 underline-offset-2 hover:text-brand-strong";
      // Pages of the app open in place; everything else leaves in a new tab.
      return node.href.startsWith("/") ? (
        <Link to={node.href} className={classes}>
          {renderInline(node.children)}
        </Link>
      ) : (
        <a
          href={node.href}
          target={node.href.startsWith("#") ? undefined : "_blank"}
          rel="noreferrer noopener"
          className={classes}
        >
          {renderInline(node.children)}
        </a>
      );
    }
  }
}
