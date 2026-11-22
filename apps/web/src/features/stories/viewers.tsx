import { Link } from "@tanstack/react-router";
import { toMillis } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { compactAge } from "#/lib/format";
import { useStoryViewers } from "./api";
import { closeStoryViewer } from "./viewer-store";

/** Who watched one of the viewer's own stories and how they reacted, most recent first. */
export function ViewersDialog({ storyId, onClose }: { storyId: string; onClose(): void }) {
  const { t } = useTranslation("stories");
  const query = useStoryViewers(storyId);
  const viewers = query.data?.pages.flatMap((page) => page.viewers) ?? [];
  const totals = query.data?.pages[0];

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("viewers.title")}
      description={
        totals
          ? t("viewers.summary", { views: totals.viewCount, reactions: totals.reactionCount })
          : undefined
      }
    >
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : viewers.length === 0 ? (
        <EmptyState emoji="👀" title={t("viewers.empty")} body={t("viewers.emptyBody")} />
      ) : (
        <ul className="flex flex-col" data-testid="story-viewers">
          {viewers.map(
            (viewer) =>
              viewer.user && (
                <li key={viewer.user.id}>
                  <Link
                    to="/u/$handle"
                    params={{ handle: viewer.user.handle }}
                    onClick={() => {
                      onClose();
                      closeStoryViewer();
                    }}
                    className="flex items-center gap-3 rounded-2xl px-2 py-2 hover:bg-surface-2"
                  >
                    <Avatar user={viewer.user} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold">
                        <EmojiText text={viewer.user.displayName} />
                      </span>
                      <span className="block truncate text-sm text-ink-soft">
                        @{viewer.user.handle} · {compactAge(toMillis(viewer.viewedAt))}
                      </span>
                    </span>
                    {viewer.reaction && (
                      <span
                        className="text-2xl leading-none"
                        role="img"
                        aria-label={t("viewers.reacted", { emoji: viewer.reaction })}
                      >
                        {viewer.reaction}
                      </span>
                    )}
                  </Link>
                </li>
              ),
          )}
          <LoadMore query={query} />
        </ul>
      )}
    </Dialog>
  );
}
