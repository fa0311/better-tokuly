import { Link } from "@tanstack/react-router";
import type { shorts } from "@tokuly/api";
import { captionParts } from "@tokuly/core/shorts";
import { EmojiText } from "#/features/emoji";

/**
 * A short's caption with its hashtags leading to their pages and its
 * mentions to the people they name. Inline content: the surrounding element
 * decides size, colour and clamping.
 */
export function Caption({
  short,
  linkClassName = "font-black",
}: {
  short: Pick<shorts.Short, "caption" | "hashtags" | "mentions">;
  linkClassName?: string;
}) {
  return captionParts(short.caption, short.hashtags, short.mentions).map((part, index) => {
    const key = `${index}:${part.text}`;
    if (part.kind === "hashtag")
      return (
        <Link key={key} to="/shorts/tags/$tag" params={{ tag: part.tag }} className={linkClassName}>
          {part.text}
        </Link>
      );
    if (part.kind === "mention")
      return (
        <Link key={key} to="/u/$handle" params={{ handle: part.handle }} className={linkClassName}>
          {part.text}
        </Link>
      );
    return <EmojiText key={key} text={part.text} />;
  });
}
