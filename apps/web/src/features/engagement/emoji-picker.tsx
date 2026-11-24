import type { ReactElement } from "react";
import { EmojiPicker as SharedEmojiPicker } from "#/features/emoji";

export interface EmojiPickerProps {
  /** Called with the emoji the person tapped (`:name:` for a custom one); the picker closes. */
  onPick(emoji: string): void;
  /** The element that opens the picker. It must forward its props and ref, which `Button` and plain DOM elements do. */
  trigger: ReactElement<Record<string, unknown>>;
  /** Where the panel opens relative to the trigger. Default: above it. */
  side?: "top" | "bottom";
  /** Told when the panel opens and closes (a story pauses meanwhile). */
  onOpenChange?(open: boolean): void;
  /**
   * Offer custom emoji too, written `:name:`. Engagement reactions take them
   * (where the subject's owner allows: `ReactionRules.customEmoji`); the other
   * fields that use this picker hold one Unicode emoji, so the default is false.
   */
  custom?: boolean;
  /** Leave out custom emoji marked sensitive (a subject whose owner does not take them). */
  hideSensitive?: boolean;
  /** Offer only these emoji: a subject that carries as many different emoji as it may. */
  only?: string[];
}

/**
 * The emoji picker for a reaction (or any field that holds one emoji): the
 * app's picker from `features/emoji` — the person's deck, what they used
 * recently, every group, search by name. With `custom` it offers the custom
 * emoji next to the Unicode ones, as the reaction bar does.
 */
export function EmojiPicker({
  onPick,
  trigger,
  side,
  onOpenChange,
  custom = false,
  hideSensitive,
  only,
}: EmojiPickerProps) {
  return (
    <SharedEmojiPicker
      unicodeOnly={!custom}
      hideSensitive={hideSensitive}
      only={only}
      trigger={trigger}
      side={side}
      onOpenChange={onOpenChange}
      onPick={(emoji) => onPick(emoji)}
    />
  );
}
