import { useTranslation } from "react-i18next";
import { type CatalogEmoji, Emoji, EmojiImage } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { isCustomEmoji, type ReactionPicture } from "./model";

/** The picture the server described, in the shape the emoji feature draws. */
function asCatalogEmoji(picture: ReactionPicture): CatalogEmoji {
  const stills =
    picture.stills.length > 0
      ? picture.stills
      : picture.staticUrl
        ? [{ size: Number.MAX_SAFE_INTEGER, url: picture.staticUrl }]
        : [];
  return {
    id: picture.name,
    name: picture.name,
    url: picture.url,
    stills,
    animated: picture.animated,
    category: "",
    aliases: [],
    sensitive: picture.sensitive,
    localOnly: false,
    addedAt: 0,
  };
}

/** How an emoji of a reaction is read out and copied: the shortcode for a custom one. */
export function reactionLabel(emoji: string, custom?: ReactionPicture): string {
  return custom ? `:${custom.name}:` : emoji;
}

export interface ReactionEmojiProps {
  /** As reactions write it: the Unicode emoji, or `:name:`. */
  emoji: string;
  /** The server's description of a custom emoji, when it sent one. */
  custom?: ReactionPicture;
  /** The height it is drawn at, in CSS pixels. Default 20. */
  size?: number;
  className?: string;
}

/**
 * One emoji of a reaction. A custom emoji is drawn from what the server sent
 * (it moves only on screen and only without reduced motion, its still
 * otherwise; sensitive ones are blurred); one that left the catalogue is
 * its shortcode as text, struck through. Before the server described it (a
 * reaction just picked) the catalogue draws it.
 */
export function ReactionEmoji({ emoji, custom, size = 20, className }: ReactionEmojiProps) {
  const { t } = useTranslation("engagement");
  if (custom && !custom.available)
    return (
      <span
        title={t("reactions.unavailable")}
        className={cn("text-xs font-bold text-ink-faint line-through", className)}
      >
        :{custom.name}:
      </span>
    );
  if (custom)
    return (
      <EmojiImage
        emoji={asCatalogEmoji(custom)}
        size={size}
        className={cn("inline-block w-auto", className)}
      />
    );
  if (isCustomEmoji(emoji)) return <Emoji name={emoji} size={size} className={className} />;
  return <span className={className}>{emoji}</span>;
}
