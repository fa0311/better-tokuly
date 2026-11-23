import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { common, live } from "@tokuly/api";
import { Timer } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { useMe } from "#/stores/session";
import { liveKeys, nextCursor, streamClient } from "../api";
import { ROLE_KEYS } from "../labels";
import { formatCount } from "../parts";
import { RestrictDialog } from "./moderation";

/** Who is watching right now, for the streamer and the channel's moderators. */
export function ViewerList({ stream }: { stream: live.Stream }) {
  const { t, i18n } = useTranslation("live");
  const me = useMe();
  const streamId = stream.summary?.id ?? "";
  const [restricting, setRestricting] = useState<common.UserSummary | null>(null);
  const query = useInfiniteQuery({
    queryKey: liveKeys.viewers(streamId),
    queryFn: ({ pageParam }) =>
      streamClient().listViewers({ streamId, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    refetchInterval: 15_000,
  });
  const viewers = query.data?.pages.flatMap((page) => page.viewers) ?? [];
  const total = query.data?.pages[0]?.viewerCount ?? 0n;
  if (query.isPending) return <PaneSpinner />;
  return (
    <div>
      <p className="px-1 pb-2 text-sm font-bold text-ink-soft">
        {t("viewersList.total", {
          formatted: formatCount(total, i18n.language),
          signedIn: viewers.length,
        })}
      </p>
      {viewers.length === 0 ? (
        <EmptyState
          emoji="👀"
          title={t("viewersList.emptyTitle")}
          body={t("viewersList.emptyBody")}
        />
      ) : (
        <ul className="flex flex-col">
          {viewers.map((viewer) => {
            const user = viewer.user;
            if (!user) return null;
            const roleKey = ROLE_KEYS[viewer.role];
            return (
              <li key={user.id} className="flex items-center gap-3 rounded-2xl px-2 py-2">
                <Avatar user={user} size="sm" />
                <Link to="/u/$handle" params={{ handle: user.handle }} className="min-w-0 flex-1">
                  <span className="block truncate font-bold">
                    <EmojiText text={user.displayName} />
                  </span>
                  <span className="block truncate text-sm text-ink-soft">
                    @{user.handle}
                    {roleKey && ` · ${t(`chatRole.${roleKey}`)}`}
                  </span>
                </Link>
                {user.id !== me?.user?.id && (
                  <button
                    type="button"
                    aria-label={t("moderation.restrictUser", { name: user.displayName })}
                    onClick={() => setRestricting(user)}
                    className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-danger"
                  >
                    <Timer className="size-4" />
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <LoadMore query={query} />
      <RestrictDialog
        channelId={stream.summary?.channel?.id ?? ""}
        user={restricting ?? undefined}
        open={restricting !== null}
        onOpenChange={(open) => !open && setRestricting(null)}
      />
    </div>
  );
}

export function ViewersDialog({
  stream,
  open,
  onOpenChange,
}: {
  stream: live.Stream;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("live");
  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={t("viewersList.title")}>
      {open && <ViewerList stream={stream} />}
    </Dialog>
  );
}
