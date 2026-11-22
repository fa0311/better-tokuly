import { Popover } from "@base-ui/react/popover";
import { type ReactElement, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "#/components/ui/dialog";
import { cn } from "#/lib/cn";
import { EMOJI_GROUPS, recentEmoji, rememberEmoji } from "./emoji";

type GroupKey = "recent" | (typeof EMOJI_GROUPS)[number]["key"];

/** The grid of emoji with its group tabs. Picking one records it under "recent". */
export function EmojiPanel({ onPick }: { onPick(emoji: string): void }) {
  const { t } = useTranslation("chat");
  const [recent] = useState(recentEmoji);
  const [group, setGroup] = useState<GroupKey>(recent.length > 0 ? "recent" : "smileys");
  const shown =
    group === "recent"
      ? recent
      : (EMOJI_GROUPS.find((candidate) => candidate.key === group)?.emoji ?? []);
  const tabs: { key: GroupKey; icon: string }[] = [
    ...(recent.length > 0 ? [{ key: "recent" as const, icon: "🕘" }] : []),
    ...EMOJI_GROUPS,
  ];

  return (
    <div className="flex min-h-0 flex-col" data-testid="emoji-picker">
      <div
        role="tablist"
        aria-label={t("emoji.groups")}
        className="scrollbar-none flex shrink-0 gap-1 overflow-x-auto pb-2"
      >
        {tabs.map((tab) => (
          <button
            key={tab.key}
            type="button"
            role="tab"
            aria-selected={tab.key === group}
            aria-label={t(`emoji.group.${tab.key}`)}
            onClick={() => setGroup(tab.key)}
            className={cn(
              "grid size-10 shrink-0 place-items-center rounded-xl text-xl transition-colors",
              tab.key === group ? "bg-brand-soft" : "hover:bg-surface-2",
            )}
          >
            {tab.icon}
          </button>
        ))}
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-7 content-start gap-0.5 overflow-y-auto sm:grid-cols-8">
        {shown.map((emoji) => (
          <button
            key={emoji}
            type="button"
            aria-label={emoji}
            onClick={() => {
              rememberEmoji(emoji);
              onPick(emoji);
            }}
            className="grid aspect-square place-items-center rounded-xl text-2xl transition-transform hover:bg-surface-2 active:scale-90"
          >
            {emoji}
          </button>
        ))}
      </div>
    </div>
  );
}

/** The emoji picker as a dialog (a sheet on phones): for reacting to a message. */
export function EmojiDialog({
  open,
  onOpenChange,
  title,
  onPick,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  onPick(emoji: string): void;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      className="h-[26rem] sm:w-[26rem]"
    >
      <div className="flex h-full flex-col">
        <EmojiPanel
          onPick={(emoji) => {
            onOpenChange(false);
            onPick(emoji);
          }}
        />
      </div>
    </Dialog>
  );
}

/** The emoji picker floating next to its trigger: for typing emoji with a keyboard and mouse. */
export function EmojiPopover({
  trigger,
  onPick,
}: {
  trigger: ReactElement<Record<string, unknown>>;
  onPick(emoji: string): void;
}) {
  const { t } = useTranslation("chat");
  const [open, setOpen] = useState(false);
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger render={trigger} />
      <Popover.Portal>
        <Popover.Positioner
          side="top"
          align="end"
          sideOffset={10}
          collisionPadding={12}
          className="z-50 outline-none"
        >
          <Popover.Popup
            aria-label={t("emoji.title")}
            className="sticker-lg flex h-[21rem] max-h-[var(--available-height)] w-[22rem] origin-[var(--transform-origin)] flex-col rounded-[1.5rem] bg-surface p-3 outline-none transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0"
          >
            {open && <EmojiPanel onPick={onPick} />}
          </Popover.Popup>
        </Popover.Positioner>
      </Popover.Portal>
    </Popover.Root>
  );
}
