import { Link } from "@tanstack/react-router";
import { notes } from "@tokuly/api";
import { parseNoteText } from "@tokuly/core/notes";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";

/** Links are shown without their scheme and cut after this many characters. */
const LINK_TEXT_MAX = 36;

function linkText(url: string): string {
  const bare = url.replace(/^https?:\/\/(www\.)?/i, "").replace(/\/$/, "");
  return bare.length > LINK_TEXT_MAX ? `${bare.slice(0, LINK_TEXT_MAX)}…` : bare;
}

const LINK = "font-bold text-brand hover:underline";

/**
 * A note's text with its hashtags, mentions and links made tappable.
 * Hashtags open their timeline; an `@handle` links to the profile only when
 * the server found that user (the note's `mentions`); links open in a new tab.
 */
export function NoteText({
  note,
  className,
}: {
  note: Pick<notes.Note, "text" | "mentions">;
  className?: string;
}) {
  if (!note.text) return null;
  const mentioned = new Set(note.mentions.map((mention) => mention.handle));
  return (
    <p className={cn("break-words whitespace-pre-wrap", className)}>
      {parseNoteText(note.text).map((segment, index) => {
        switch (segment.type) {
          case "hashtag":
            return (
              <Link key={index} to="/tag/$tag" params={{ tag: segment.tag }} className={LINK}>
                {segment.text}
              </Link>
            );
          case "mention":
            return mentioned.has(segment.handle) ? (
              <Link
                key={index}
                to="/u/$handle"
                params={{ handle: segment.handle }}
                className={LINK}
              >
                {segment.text}
              </Link>
            ) : (
              segment.text
            );
          case "url":
            return (
              <a
                key={index}
                href={segment.url}
                target="_blank"
                rel="noopener noreferrer nofollow"
                className={LINK}
                title={segment.url}
              >
                {linkText(segment.url)}
              </a>
            );
          default:
            // Plain text: its `:shortcode:`s are drawn as custom emoji.
            return <EmojiText key={index} text={segment.text} />;
        }
      })}
    </p>
  );
}

/** Keyframes of the text effects; they only run when the reader has not asked for less motion. */
const EFFECT_CSS = `
@media (prefers-reduced-motion: no-preference) {
  :root:not([data-dopa="0"]) .note-fx-tada { animation: note-fx-tada 1s linear infinite; }
  :root:not([data-dopa="0"]) .note-fx-jelly { animation: note-fx-jelly 1s linear infinite; }
  :root:not([data-dopa="0"]) .note-fx-jump { animation: note-fx-jump 0.75s linear infinite; }
  :root:not([data-dopa="0"]) .note-fx-bounce { animation: note-fx-bounce 0.75s linear infinite; transform-origin: center bottom; }
  :root:not([data-dopa="0"]) .note-fx-shake { animation: note-fx-shake 0.5s ease infinite; }
  :root:not([data-dopa="0"]) .note-fx-spin { animation: note-fx-spin 1.5s linear infinite; }
  :root:not([data-dopa="0"]) .note-fx-rainbow { animation: note-fx-rainbow 1s linear infinite; }
  :root:not([data-dopa="0"]) .note-fx-sparkle { animation: note-fx-sparkle 1.5s ease-in-out infinite; }
}
.note-fx { display: inline-block; }
.note-fx-x2 { font-size: 200%; line-height: 1.2; }
@keyframes note-fx-tada { 0%,100% { transform: scale(1); } 10%,20% { transform: scale(0.9) rotate(-3deg); } 30%,50%,70%,90% { transform: scale(1.1) rotate(3deg); } 40%,60%,80% { transform: scale(1.1) rotate(-3deg); } }
@keyframes note-fx-jelly { 0%,100% { transform: scale(1, 1); } 30% { transform: scale(1.25, 0.75); } 40% { transform: scale(0.75, 1.25); } 50% { transform: scale(1.15, 0.85); } 65% { transform: scale(0.95, 1.05); } 75% { transform: scale(1.05, 0.95); } }
@keyframes note-fx-jump { 0%,50%,100% { transform: translateY(0); } 25% { transform: translateY(-16px); } }
@keyframes note-fx-bounce { 0%,50%,100% { transform: translateY(0) scale(1, 1); } 25% { transform: translateY(-16px) scale(1, 1); } 40% { transform: translateY(0) scale(1.5, 0.75); } }
@keyframes note-fx-shake { 0%,100% { transform: translate(0, 0); } 25% { transform: translate(-2px, 1px); } 50% { transform: translate(2px, -1px); } 75% { transform: translate(-1px, -2px); } }
@keyframes note-fx-spin { to { transform: rotate(360deg); } }
@keyframes note-fx-rainbow { from { filter: hue-rotate(0deg) saturate(2); } to { filter: hue-rotate(360deg) saturate(2); } }
@keyframes note-fx-sparkle { 0%,100% { filter: brightness(1); } 50% { filter: brightness(1.6) drop-shadow(0 0 4px gold); } }
`;

const EFFECT_CLASS: Record<number, string> = {
  [notes.TextEffectKind.TADA]: "note-fx-tada",
  [notes.TextEffectKind.JELLY]: "note-fx-jelly",
  [notes.TextEffectKind.JUMP]: "note-fx-jump",
  [notes.TextEffectKind.BOUNCE]: "note-fx-bounce",
  [notes.TextEffectKind.SHAKE]: "note-fx-shake",
  [notes.TextEffectKind.SPIN]: "note-fx-spin",
  [notes.TextEffectKind.RAINBOW]: "note-fx-rainbow",
  [notes.TextEffectKind.SPARKLE]: "note-fx-sparkle",
  [notes.TextEffectKind.X2]: "note-fx-x2",
};

function Inlines({ nodes, inLink }: { nodes: notes.TextInline[]; inLink?: boolean }) {
  return nodes.map((node, index) => <InlineNode key={index} node={node} inLink={inLink} />);
}

function InlineNode({ node, inLink }: { node: notes.TextInline; inLink?: boolean }) {
  const value = node.inline;
  switch (value.case) {
    case "text":
      return <EmojiText text={value.value} />;
    case "code":
      return (
        <code className="rounded-md bg-surface-2 px-1 py-0.5 font-mono text-[0.9em]">
          {value.value}
        </code>
      );
    case "span": {
      const children = <Inlines nodes={value.value.children} inLink={inLink} />;
      switch (value.value.style) {
        case notes.TextStyle.BOLD:
          return <strong className="font-black">{children}</strong>;
        case notes.TextStyle.ITALIC:
          return <em>{children}</em>;
        case notes.TextStyle.STRIKE:
          return <s>{children}</s>;
        case notes.TextStyle.SMALL:
          return <small className="text-[0.8em] text-ink-soft">{children}</small>;
        default:
          return children;
      }
    }
    case "link": {
      const link = value.value;
      if (inLink) return <Inlines nodes={link.label} inLink />;
      return (
        <a
          href={link.url}
          target="_blank"
          rel="noopener noreferrer nofollow"
          className={LINK}
          title={link.url}
        >
          {link.label.length > 0 ? <Inlines nodes={link.label} inLink /> : linkText(link.url)}
        </a>
      );
    }
    case "mention": {
      const mention = value.value;
      return mention.userId && !inLink ? (
        <Link to="/u/$handle" params={{ handle: mention.handle }} className={LINK}>
          @{mention.handle}
        </Link>
      ) : (
        <>@{mention.handle}</>
      );
    }
    case "hashtag":
      return inLink ? (
        <>#{value.value.text}</>
      ) : (
        <Link to="/tag/$tag" params={{ tag: value.value.tag }} className={LINK}>
          #{value.value.text}
        </Link>
      );
    case "emoji": {
      const emoji = value.value;
      return emoji.image?.url ? (
        <img
          src={emoji.image.url}
          alt={`:${emoji.name}:`}
          title={`:${emoji.name}:`}
          className="inline-block h-[1.4em] w-auto align-[-0.3em]"
          loading="lazy"
        />
      ) : (
        <EmojiText text={`:${emoji.name}:`} />
      );
    }
    case "effect":
      return (
        <span className={cn("note-fx", EFFECT_CLASS[value.value.kind])}>
          <Inlines nodes={value.value.children} inLink={inLink} />
        </span>
      );
    default:
      return null;
  }
}

function Paragraphs({ paragraphs }: { paragraphs: notes.TextParagraph[] }) {
  return paragraphs.map((paragraph, index) => (
    <p key={index} className="break-words whitespace-pre-wrap">
      <Inlines nodes={paragraph.inlines} />
    </p>
  ));
}

/**
 * A note's text as the server parsed it (`Note.body`): styles, code,
 * quotes, links, mentions, hashtags, custom emoji and effects. Notes
 * without a tree (older ones) fall back to the plain text.
 */
export function NoteRichText({
  note,
  className,
}: {
  note: Pick<notes.Note, "text" | "mentions" | "body">;
  className?: string;
}) {
  const blocks = note.body?.blocks ?? [];
  if (blocks.length === 0) return <NoteText note={note} className={className} />;
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      <style href="tokuly-note-effects" precedence="default">
        {EFFECT_CSS}
      </style>
      {blocks.map((block, index) => {
        const value = block.block;
        switch (value.case) {
          case "paragraph":
            return <Paragraphs key={index} paragraphs={[value.value]} />;
          case "quote":
            return (
              <blockquote key={index} className="border-l-4 border-line pl-3 text-ink-soft">
                <Paragraphs paragraphs={value.value.paragraphs} />
              </blockquote>
            );
          case "center":
            return (
              <div key={index} className="text-center">
                <Paragraphs paragraphs={value.value.paragraphs} />
              </div>
            );
          case "code":
            return (
              <pre
                key={index}
                data-language={value.value.language || undefined}
                className="overflow-x-auto rounded-xl bg-surface-2 p-3 font-mono text-sm"
              >
                <code>{value.value.code}</code>
              </pre>
            );
          default:
            return null;
        }
      })}
    </div>
  );
}
