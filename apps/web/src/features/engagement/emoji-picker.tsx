import { Popover } from "@base-ui/react/popover";
import { type ReactElement, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { EMOJI_GROUPS, type EmojiGroup, quickEmoji, rememberEmoji } from "./emoji-data";

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
 * A small emoji picker for reactions: the person's recent emoji in a row of
 * one-tap choices, then every emoji by group. It floats next to its trigger
 * on any screen size.
 */
export function EmojiPicker({ onPick, trigger, side = "top", onOpenChange }: EmojiPickerProps) {
  const { t } = useTranslation("engagement");
  const [open, setOpenState] = useState(false);
  const setOpen = (next: boolean) => {
    setOpenState(next);
    onOpenChange?.(next);
  };
  const pick = (emoji: string) => {
    rememberEmoji(emoji);
    setOpen(false);
    onPick(emoji);
  };

  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger render={trigger} />
      <Popover.Portal>
        <Popover.Positioner
          side={side}
          align="center"
          sideOffset={8}
          collisionPadding={10}
          className="z-50 outline-none"
        >
          <Popover.Popup
            aria-label={t("emoji.title")}
            data-testid="emoji-picker"
            className={cn(
              "sticker-lg flex max-h-[min(22rem,var(--available-height))] w-[min(21.5rem,calc(100vw-1.25rem))] origin-[var(--transform-origin)] flex-col overflow-hidden rounded-[1.5rem] bg-surface outline-none",
              "transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0",
            )}
          >
            {open && <PickerPanel onPick={pick} />}
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}

function PickerPanel({ onPick }: { onPick(emoji: string): void }) {
  const { t } = useTranslation("engagement");
  const [groupId, setGroupId] = useState<EmojiGroup["id"]>("smileys");
  // Read once per opening: the row must not reshuffle while a finger is on its way.
  const [quick] = useState(() => quickEmoji());
  const group = EMOJI_GROUPS.find((item) => item.id === groupId) ?? EMOJI_GROUPS[0]!;

  return (
    <>
      {/* biome-ignore lint/a11y/useSemanticElements: a row of emoji buttons, not a form group. */}
      <div
        className="flex shrink-0 items-center justify-between gap-0.5 border-b-2 border-line px-2 py-2"
        role="group"
        aria-label={t("emoji.quick")}
      >
        {quick.map((emoji) => (
          <EmojiButton key={emoji} emoji={emoji} onPick={onPick} large />
        ))}
      </div>
      <div role="tablist" aria-label={t("emoji.title")} className="flex shrink-0 gap-0.5 px-2 pt-2">
        {EMOJI_GROUPS.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={item.id === groupId}
            aria-label={t(`emoji.groups.${item.id}`)}
            onClick={() => setGroupId(item.id)}
            className={cn(
              "grid h-9 flex-1 place-items-center rounded-xl text-lg transition-colors",
              item.id === groupId
                ? "bg-brand-soft"
                : "opacity-60 hover:bg-surface-2 hover:opacity-100",
            )}
          >
            <span aria-hidden>{item.icon}</span>
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        aria-label={t(`emoji.groups.${group.id}`)}
        className="grid min-h-0 flex-1 grid-cols-8 content-start gap-0.5 overflow-y-auto p-2"
      >
        {group.emoji.map((emoji) => (
          <EmojiButton key={emoji} emoji={emoji} onPick={onPick} />
        ))}
      </div>
    </>
  );
}

function EmojiButton({
  emoji,
  onPick,
  large,
}: {
  emoji: string;
  onPick(emoji: string): void;
  large?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={emoji}
      onClick={() => onPick(emoji)}
      className={cn(
        "grid aspect-square place-items-center rounded-xl leading-none transition-transform hover:bg-surface-2 active:scale-125",
        large ? "size-9 text-2xl" : "text-[1.35rem]",
      )}
    >
      <span aria-hidden>{emoji}</span>
    </button>
  );
}
