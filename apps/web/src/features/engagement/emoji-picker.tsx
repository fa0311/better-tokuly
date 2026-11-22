import type { ReactElement } from "react";
import { EmojiPicker as SharedEmojiPicker } from "#/features/emoji";

export interface EmojiPickerProps {
  /** Called with the emoji the person tapped; the picker closes. */
  onPick(emoji: string): void;
  /** The element that opens the picker. It must forward its props and ref, which `Button` and plain DOM elements do. */
  trigger: ReactElement<Record<string, unknown>>;
  /** Where the panel opens relative to the trigger. Default: above it. */
  side?: "top" | "bottom";
  /** Told when the panel opens and closes (a story pauses meanwhile). */
  onOpenChange?(open: boolean): void;
}

/**
 * The emoji picker for a reaction (or any field that holds one emoji): the
 * app's picker from `features/emoji` — the person's deck, what they used
 * recently, every group, search by name — showing Unicode emoji only.
 *
 * Custom emoji are left out because the reaction RPCs take exactly one
 * Unicode emoji (`EMOJI_INVALID` otherwise). Once the backend accepts
 * `:shortcode:` reactions, dropping `unicodeOnly` here is the whole change.
 */
export function EmojiPicker({ onPick, trigger, side, onOpenChange }: EmojiPickerProps) {
  return (
    <SharedEmojiPicker
      unicodeOnly
      trigger={trigger}
      side={side}
      onOpenChange={onOpenChange}
      onPick={(emoji) => onPick(emoji)}
    />
  );
}
