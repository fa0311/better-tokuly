import { Fragment } from "react";
import { useCatalog } from "./catalog";
import { EmojiImage } from "./emoji";
import { splitEmojiText } from "./text";

export interface EmojiTextProps {
  /** Plain text: a message, a display name, a profile line. */
  text: string;
  /** Classes for each emoji picture (they are as tall as the text by default). */
  emojiClassName?: string;
  /** The pixel height emoji are drawn at, when it is not the usual text size: picks the rendition. */
  emojiSize?: number;
}

/**
 * Text with its `:shortcode:`s drawn as custom emoji:
 *
 *   <p><EmojiText text={message.text} /></p>
 *   <span className="font-black"><EmojiText text={user.displayName} /></span>
 *
 * It renders inline content (no wrapper), so the surrounding element decides
 * the typography. Codes of emoji that do not exist stay as they were typed,
 * Unicode emoji are left alone, and everything else is put into the page as
 * text, never as markup. When the catalogue changes, the text draws again.
 */
export function EmojiText({ text, emojiClassName, emojiSize }: EmojiTextProps) {
  const catalog = useCatalog();
  // Most text has no colon at all: skip the work.
  if (!text.includes(":") || catalog.emojis.length === 0) return text;
  const segments = splitEmojiText(text, (name) => catalog.byName.has(name));
  let offset = 0;
  return segments.map((segment) => {
    const key = offset;
    offset += segment.kind === "text" ? segment.text.length : segment.raw.length;
    if (segment.kind === "text") return <Fragment key={key}>{segment.text}</Fragment>;
    // Present: `splitEmojiText` only names emoji the catalogue has.
    const entry = catalog.byName.get(segment.name);
    return entry ? (
      <EmojiImage key={key} emoji={entry} size={emojiSize} className={emojiClassName} />
    ) : (
      <Fragment key={key}>{segment.raw}</Fragment>
    );
  });
}
