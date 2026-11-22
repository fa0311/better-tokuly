import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Clock, Flame, MoreHorizontal, Pause, Play, Trash2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu } from "#/components/ui/menu";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { submitSearch, useHistoryActions, useRecentSearches, useTrending } from "./api";
import { ageLabel } from "./format";
import { isKind, kindSlug } from "./kinds";

const heading = "flex min-w-0 flex-1 items-center gap-2 text-lg font-black";

/** Recent searches shown before "show all" is pressed. */
const RECENT_PREVIEW = 6;

/**
 * The person's recent searches: run one again, forget one, forget all, and
 * pause the history (while paused nothing new is remembered and their
 * searches stop counting towards trends).
 */
export function RecentPanel({ className }: { className?: string }) {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const recent = useRecentSearches();
  const { forget, clear, setPaused } = useHistoryActions();
  const [confirming, setConfirming] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const searches = recent.data?.searches ?? [];
  const shown = expanded ? searches : searches.slice(0, RECENT_PREVIEW);
  const paused = recent.data?.historyPaused ?? false;
  if (recent.isPending || recent.isError) return null;

  return (
    <section aria-label={t("recent.title")} className={className}>
      <header className="mb-1 flex items-center gap-2 px-1">
        <h2 className={heading}>
          <Clock className="size-5 shrink-0 text-ink-soft" />
          <span className="truncate">{t("recent.title")}</span>
          {paused && <span className="shrink-0 rounded-full bg-sun px-2 py-0.5 text-[11px] font-black text-[#17122b]">{t("recent.paused")}</span>}
        </h2>
        <Menu
          trigger={
            <Button variant="ghost" size="icon-sm" aria-label={t("recent.menu")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            paused
              ? { label: t("recent.resume"), icon: <Play />, onSelect: () => setPaused.mutate(false, { onSuccess: () => toast.success(t("recent.resumed")) }) }
              : { label: t("recent.pause"), icon: <Pause />, onSelect: () => setPaused.mutate(true, { onSuccess: () => toast.success(t("recent.pausedToast")) }) },
            { label: t("recent.clear"), icon: <Trash2 />, danger: true, disabled: searches.length === 0, onSelect: () => setConfirming(true) },
          ]}
        />
      </header>
      {searches.length === 0 ? (
        <p className="px-1 py-2 text-sm leading-relaxed text-ink-soft">{t(paused ? "recent.emptyPaused" : "recent.empty")}</p>
      ) : (
        <ul className="flex flex-col">
          {shown.map((item) => (
            <li key={item.id} className="flex items-center rounded-2xl hover:bg-surface-2">
              <Link
                to="/search"
                search={{ q: item.query, kind: isKind(item.kind) ? kindSlug(item.kind) : undefined }}
                onClick={() => submitSearch(queryClient, item.query)}
                className="flex min-h-11 min-w-0 flex-1 items-center gap-3 px-3 py-1.5"
              >
                <span className="min-w-0 flex-1 truncate font-bold">{item.query}</span>
                {isKind(item.kind) && <span className="shrink-0 rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-bold text-ink-soft">{t(`kinds.${kindSlug(item.kind)}`)}</span>}
                <span className="shrink-0 text-xs text-ink-faint">{ageLabel(item.searchedAt)}</span>
              </Link>
              <button
                type="button"
                aria-label={t("recent.forget", { query: item.query })}
                onClick={() => forget.mutate(item.id)}
                className="mr-1 grid size-9 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-3 hover:text-ink"
              >
                <X className="size-4" />
              </button>
            </li>
          ))}
        </ul>
      )}
      {searches.length > RECENT_PREVIEW && (
        <button type="button" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)} className="mt-1 ml-3 inline-flex h-9 items-center text-sm font-bold text-brand hover:underline">
          {expanded ? t("recent.showLess") : t("recent.showAll", { count: searches.length })}
        </button>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("recent.clearTitle")}
        description={t("recent.clearBody")}
        confirmLabel={t("recent.clearConfirm")}
        danger
        loading={clear.isPending}
        onConfirm={() =>
          clear.mutate(undefined, {
            onSuccess: () => toast.success(t("recent.cleared")),
            onSettled: () => setConfirming(false),
          })
        }
      />
    </section>
  );
}

/** What many people are searching for right now, as a ranked list. */
export function TrendingPanel({ limit = 10, className }: { limit?: number; className?: string }) {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const trending = useTrending(limit);
  const queries = trending.data?.queries ?? [];
  if (trending.isError) return null;
  return (
    <section aria-label={t("trending.title")} className={className}>
      <header className="mb-2 flex items-center gap-2 px-1">
        <h2 className={heading}>
          <Flame className="size-5 shrink-0 text-coral" />
          <span className="truncate">{t("trending.title")}</span>
        </h2>
      </header>
      {trending.isPending ? (
        <div className="flex flex-col gap-2" aria-busy>
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : queries.length === 0 ? (
        <p className="px-1 py-2 text-sm leading-relaxed text-ink-soft">{t("trending.empty")}</p>
      ) : (
        <ol className="flex flex-col">
          {queries.map((item) => (
            <li key={item.query} className="min-w-0">
              <Link to="/search" search={{ q: item.query }} onClick={() => submitSearch(queryClient, item.query)} className="flex min-h-11 items-center gap-3 rounded-2xl px-2 py-1.5 hover:bg-surface-2">
                <span className={cn("grid size-7 shrink-0 place-items-center rounded-lg text-sm font-black tabular-nums", item.rank <= 3 ? "bg-pink text-white" : "bg-surface-2 text-ink-soft")}>{item.rank}</span>
                <span className="min-w-0 flex-1 truncate font-bold">{item.query}</span>
              </Link>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
