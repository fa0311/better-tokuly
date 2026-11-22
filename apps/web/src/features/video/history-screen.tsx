import { type InfiniteData, useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toMillis, video } from "@tokuly/api";
import { History, Trash2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { nextCursor, playbackClient, videoKeys } from "./api";
import { VideoCard, VideoCardSkeleton } from "./video-card";

type HistoryPages = InfiniteData<video.ListWatchHistoryResponse>;

/** "今日" / "昨日" / a date: the heading a history entry is filed under. */
function dayLabel(at: number, language: string, t: (key: string) => string, now = new Date()): string {
  const day = new Date(at);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (at >= startOfToday) return t("history.today");
  if (at >= startOfToday - 86_400_000) return t("history.yesterday");
  return day.toLocaleDateString(language, { year: day.getFullYear() === now.getFullYear() ? undefined : "numeric", month: "long", day: "numeric", weekday: "short" });
}

/** `/watch/history`: what the viewer watched, day by day; entries can be removed, everything cleared, recording paused. */
export function HistoryScreen() {
  const { t, i18n } = useTranslation("video");
  const queryClient = useQueryClient();
  const [clearing, setClearing] = useState(false);
  const history = useInfiniteQuery({
    queryKey: videoKeys.history,
    queryFn: ({ pageParam }) => playbackClient().listWatchHistory({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
  });
  const paused = history.data?.pages[0]?.paused ?? false;
  const setPages = (change: (page: video.ListWatchHistoryResponse) => video.ListWatchHistoryResponse) =>
    queryClient.setQueryData<HistoryPages>(videoKeys.history, (data) => (data ? { ...data, pages: data.pages.map(change) } : data));
  const onError = (error: unknown) => {
    toast.error(errorMessage(error));
    void queryClient.invalidateQueries({ queryKey: videoKeys.history });
  };

  const remove = useMutation({
    mutationFn: (videoId: string) => playbackClient().removeWatchHistoryItem({ videoId }),
    onMutate: (videoId) => setPages((page) => ({ ...page, items: page.items.filter((item) => item.video?.id !== videoId) })),
    onSuccess: () => toast.success(t("history.removed")),
    onError,
  });
  const clear = useMutation({
    mutationFn: () => playbackClient().clearWatchHistory({}),
    onSuccess: () => {
      setClearing(false);
      setPages((page) => ({ ...page, items: [], page: undefined }));
      // Resume positions went with it: cards elsewhere lose their progress bars.
      void queryClient.invalidateQueries({ queryKey: videoKeys.feeds });
      toast.success(t("history.cleared"));
    },
    onError: (error) => {
      setClearing(false);
      onError(error);
    },
  });
  const pause = useMutation({
    mutationFn: (next: boolean) => playbackClient().setWatchHistoryPaused({ paused: next }),
    onMutate: (next) => setPages((page) => ({ ...page, paused: next })),
    onSuccess: ({ paused: now }) => toast.message(now ? t("history.pausedToast") : t("history.resumedToast")),
    onError,
  });

  const items = history.data?.pages.flatMap((page) => page.items) ?? [];
  // Entries arrive newest first; a new heading starts whenever the day changes.
  const entries = items.flatMap((item, index) => {
    if (!item.video) return [];
    const day = dayLabel(toMillis(item.lastWatchedAt), i18n.language, t);
    const previous = items[index - 1];
    return [{ summary: item.video, day, heading: !previous || dayLabel(toMillis(previous.lastWatchedAt), i18n.language, t) !== day }];
  });

  return (
    <Page
      title={t("history.title")}
      back="/watch"
      actions={
        items.length > 0 && (
          <Button size="sm" variant="ghost" className="text-danger" icon={<Trash2 className="size-4" />} onClick={() => setClearing(true)}>
            {t("history.clear")}
          </Button>
        )
      }
    >
      <div className="sticker rounded-card bg-surface p-2">
        <SwitchRow label={t("history.pause")} hint={paused ? t("history.pausedHint") : t("history.pauseHint")} checked={paused} disabled={history.isPending} onChange={(next) => pause.mutate(next)} />
      </div>

      <div className="mt-5">
        {history.isPending ? (
          <div className="flex flex-col gap-3">
            {[0, 1, 2, 3].map((row) => (
              <VideoCardSkeleton key={row} layout="row" />
            ))}
          </div>
        ) : history.isError ? (
          <EmptyState emoji="😵‍💫" title={errorMessage(history.error)} />
        ) : items.length === 0 ? (
          <EmptyState emoji="🍿" title={paused ? t("history.emptyPaused") : t("history.empty")} body={paused ? t("history.emptyPausedBody") : t("history.emptyBody")} />
        ) : (
          <div className="flex flex-col gap-3">
            {entries.map(({ summary, day, heading }) => (
              <div key={summary.id} className="flex flex-col gap-3">
                {heading && (
                  <h2 className="flex items-center gap-2 px-1 pt-2 text-sm font-black text-ink-soft">
                    <History className="size-4" />
                    {day}
                  </h2>
                )}
                <VideoCard video={summary} layout="row" source={video.TrafficSource.HISTORY} actions={[{ label: t("history.remove"), icon: <X />, onSelect: () => remove.mutate(summary.id) }]} />
              </div>
            ))}
            <LoadMore query={history} showEnd={items.length > 20} />
          </div>
        )}
      </div>

      <ConfirmDialog
        open={clearing}
        onOpenChange={setClearing}
        title={t("history.confirmClear")}
        description={t("history.confirmClearBody")}
        confirmLabel={t("history.clear")}
        danger
        loading={clear.isPending}
        onConfirm={() => clear.mutate()}
      />
    </Page>
  );
}
