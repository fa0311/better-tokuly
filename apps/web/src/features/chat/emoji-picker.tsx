import { Dialog } from "#/components/ui/dialog";
import { EmojiPickerPanel } from "#/features/emoji";

/**
 * The emoji picker as a dialog (a sheet on phones): for reacting to a
 * message. Unicode emoji only: `AddReaction` takes exactly one of those
 * (`EMOJI_INVALID` for a `:shortcode:`).
 */
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
      className="h-[30rem] sm:w-[26rem]"
    >
      <div className="-mx-4 -my-3 flex h-full flex-col" data-testid="emoji-picker">
        <EmojiPickerPanel
          unicodeOnly
          onPick={(emoji) => {
            onOpenChange(false);
            onPick(emoji);
          }}
        />
      </div>
    </Dialog>
  );
}
