import { compactCount, type Subject } from "@tokuly/core/engagement";
import { Bookmark } from "lucide-react";
import { toast } from "sonner";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useBookmark, useEngagement } from "./api";
import { useBurst } from "./burst";
import { openCollectionChooser } from "./collection-chooser-host";

export interface BookmarkButtonProps {
  subject: Subject;
  /** Show how many people saved the subject next to the icon. */
  showCount?: boolean;
  className?: string;
}

/**
 * Saves a subject for later. The first tap saves it (the toast offers to
 * file it into a collection); a tap on something already saved opens the
 * collection chooser, which is also where saving is undone.
 */
export function BookmarkButton({ subject, showCount, className }: BookmarkButtonProps) {
  const { t, i18n } = useTranslation("engagement");
  const { data } = useEngagement(subject);
  const bookmark = useBookmark(subject);
  const { fire, burst } = useBurst(["🔖", "✨"]);
  const saved = data?.bookmarked ?? false;

  const onClick = async () => {
    if (saved) {
      openCollectionChooser(subject);
      return;
    }
    fire();
    if (await bookmark.save()) {
      toast.success(t("bookmarks.saved"), {
        action: { label: t("bookmarks.choose"), onClick: () => openCollectionChooser(subject) },
      });
    }
  };

  return (
    <button
      type="button"
      aria-pressed={saved}
      aria-label={saved ? t("bookmarks.savedLabel") : t("bookmarks.save")}
      disabled={!data}
      onClick={() => void onClick()}
      className={cn(
        "relative inline-flex h-9 min-w-9 items-center justify-center gap-1.5 rounded-full px-2 text-sm font-black transition-colors disabled:opacity-50",
        saved ? "text-brand" : "text-ink-soft hover:bg-surface-2 hover:text-brand",
        className,
      )}
    >
      <Bookmark
        className={cn(
          "size-[18px] transition-transform duration-200 ease-bounce",
          saved && "scale-110 fill-current",
        )}
      />
      {showCount && data && data.bookmarkCount > 0 && (
        <span className="tabular-nums">{compactCount(data.bookmarkCount, i18n.language)}</span>
      )}
      {burst}
    </button>
  );
}
