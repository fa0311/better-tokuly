import { Link } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { parseNoteText } from "./text";

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
