import { useEffect, useRef, useState } from "react";
import { cn } from "#/lib/cn";
import { type CatalogEmoji, stillUrl, useCatalog } from "./catalog";
import { useMotionAllowed, useShowSensitive, watchVisibility } from "./prefs";

export interface EmojiImageProps {
  emoji: CatalogEmoji;
  /** The height it is drawn at, in CSS pixels: picks the rendition to load. Default 24. */
  size?: number;
  /** Sizing classes. Without any it is as tall as the text around it. */
  className?: string;
  /** Show a sensitive emoji as it is, whatever the person's setting (its own page, a moderator's table). */
  reveal?: boolean;
  /** Never move, even when the emoji is animated (a dense grid). */
  still?: boolean;
}

/**
 * The picture of a custom emoji. An animated one moves only while it is on
 * screen and only if the person did not ask for less movement; otherwise its
 * still rendition is shown. Sensitive emoji are blurred until the person
 * chooses to see them.
 */
export function EmojiImage({ emoji, size = 24, className, reveal, still }: EmojiImageProps) {
  const image = useRef<HTMLImageElement>(null);
  const motionAllowed = useMotionAllowed();
  const showSensitive = useShowSensitive();
  const mayMove = emoji.animated && motionAllowed && !still;
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    if (!mayMove || !image.current) return;
    return watchVisibility(image.current, setVisible);
  }, [mayMove]);

  const code = `:${emoji.name}:`;
  return (
    <img
      ref={image}
      src={mayMove && visible ? emoji.url : stillUrl(emoji, size)}
      // The shortcode is the text alternative, so copying a sentence keeps its emoji.
      alt={code}
      title={code}
      data-emoji={emoji.name}
      data-playing={mayMove && visible ? "" : undefined}
      loading="lazy"
      decoding="async"
      draggable={false}
      className={cn("inline-block h-[1.35em] w-auto max-w-none object-contain align-[-0.3em]", emoji.sensitive && !reveal && !showSensitive && "blur-[0.22em]", className)}
    />
  );
}

export interface EmojiProps {
  /** A custom emoji's name, with or without colons (`neko` or `:neko:`). */
  name: string;
  size?: number;
  className?: string;
}

/**
 * One custom emoji by name: `<Emoji name="neko" />`. While the catalogue has
 * no emoji of that name (not loaded yet, or there is none) the shortcode is
 * shown as text.
 */
export function Emoji({ name, size, className }: EmojiProps) {
  const catalog = useCatalog();
  const bare = name.replace(/^:|:$/g, "").toLowerCase();
  const entry = catalog.byName.get(bare);
  if (!entry) return <span className={className}>:{bare}:</span>;
  return <EmojiImage emoji={entry} size={size} className={className} />;
}
