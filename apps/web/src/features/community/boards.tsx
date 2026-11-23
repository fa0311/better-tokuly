import { useNavigate } from "@tanstack/react-router";
import { community } from "@tokuly/api";
import { Lock, PenLine, VenetianMask, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { NS, useChannels, useThreads } from "./api";
import { AnonymousNotice, ThreadComposer } from "./composer";
import { isMember } from "./model";
import { Notice, useDurationLabel } from "./parts";
import { ThreadCard } from "./thread-card";

export type BoardSort = "hot" | "new" | "top";
export type BoardWindow = "day" | "week" | "month" | "year" | "all";

export interface BoardSearch {
  channel?: string;
  sort?: BoardSort;
  window?: BoardWindow;
  tag?: string;
}

const SORTS: Record<BoardSort, community.ThreadSort> = {
  hot: community.ThreadSort.HOT,
  new: community.ThreadSort.NEW,
  top: community.ThreadSort.TOP,
};
const WINDOWS: Record<BoardWindow, community.TopWindow> = {
  day: community.TopWindow.DAY,
  week: community.TopWindow.WEEK,
  month: community.TopWindow.MONTH,
  year: community.TopWindow.YEAR,
  all: community.TopWindow.ALL,
};

export function parseBoardSearch(search: Record<string, unknown>): BoardSearch {
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const sort = text(search.sort);
  const window = text(search.window);
  return {
    channel: text(search.channel),
    sort: sort === "new" || sort === "top" ? sort : undefined,
    window: window && window in WINDOWS && window !== "all" ? (window as BoardWindow) : undefined,
    tag: text(search.tag),
  };
}

/** The boards of a community: its channels, and the threads of one (or all) in the chosen order. */
export function Boards({
  community: c,
  search,
  onSearchChange,
}: {
  community: community.Community;
  search: BoardSearch;
  onSearchChange(next: BoardSearch): void;
}) {
  const { t } = useTranslation(NS);
  const navigate = useNavigate();
  const durationLabel = useDurationLabel();
  const [composing, setComposing] = useState(false);
  const channels = useChannels(c.id);
  const channel = channels.data?.find((each) => each.id === search.channel);
  const sort = search.sort ?? "hot";
  const threads = useThreads(c.id, {
    channelId: channel?.id ?? "",
    sort: SORTS[sort],
    window: sort === "top" ? WINDOWS[search.window ?? "all"] : community.TopWindow.UNSPECIFIED,
    tag: search.tag ?? "",
  });
  const member = isMember(c);
  const canPost = channel
    ? channel.canPost
    : (channels.data?.some((each) => each.canPost) ?? false);
  const byId = new Map(channels.data?.map((each) => [each.id, each]));
  const pinned = threads.pinned ?? [];
  const list = threads.threads ?? [];

  return (
    <div>
      <nav
        aria-label={t("channel.list")}
        className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1"
      >
        <ChannelPill
          active={!channel}
          onClick={() => onSearchChange({ ...search, channel: undefined })}
          label={t("channel.all")}
        />
        {channels.data?.map((each) => (
          <ChannelPill
            key={each.id}
            active={each.id === channel?.id}
            onClick={() => onSearchChange({ ...search, channel: each.id })}
            label={each.name}
            emoji={each.emoji}
            unread={each.hasUnread}
            anonymous={each.anonymous}
            readOnly={each.posting === community.PostingPolicy.MODERATORS}
          />
        ))}
      </nav>

      {channel && (
        <div className="mt-3 flex flex-col gap-2">
          {channel.description && (
            <p className="px-1 text-sm text-ink-soft">{channel.description}</p>
          )}
          {channel.anonymous && <AnonymousNotice />}
          {channel.posting === community.PostingPolicy.MODERATORS && (
            <Notice icon={<Lock />}>{t("channel.moderatorsOnly")}</Notice>
          )}
          {channel.slowModeSeconds > 0 && (
            <p className="px-1 text-xs font-bold text-ink-faint">
              🐢 {t("slow.on", { interval: durationLabel(channel.slowModeSeconds) })}
            </p>
          )}
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Tabs
          items={(["hot", "new", "top"] as const).map((value) => ({
            value,
            label: t(`sort.${value}`),
          }))}
          value={sort}
          onChange={(next) =>
            onSearchChange({
              ...search,
              sort: next === "hot" ? undefined : next,
              window: undefined,
            })
          }
        />
        {sort === "top" && (
          <SelectField
            className="w-36"
            value={search.window ?? "all"}
            onChange={(next) =>
              onSearchChange({ ...search, window: next === "all" ? undefined : next })
            }
            options={(["day", "week", "month", "year", "all"] as const).map((value) => ({
              value,
              label: t(`window.${value}`),
            }))}
          />
        )}
        {search.tag && (
          <button
            type="button"
            onClick={() => onSearchChange({ ...search, tag: undefined })}
            className="inline-flex h-8 items-center gap-1 rounded-full bg-brand-soft px-3 text-sm font-bold text-brand-strong"
            aria-label={t("tag.clear", { tag: search.tag })}
          >
            #{search.tag}
            <X className="size-3.5" />
          </button>
        )}
        {member && canPost && (
          <Button
            size="sm"
            className="ml-auto"
            icon={<PenLine className="size-4" />}
            onClick={() => setComposing(true)}
          >
            {t("compose.new")}
          </Button>
        )}
      </div>

      <div className="mt-3 flex flex-col gap-2">
        {threads.isPending ? (
          Array.from({ length: 4 }, (_, index) => <Skeleton key={index} className="h-28" />)
        ) : pinned.length + list.length === 0 ? (
          <EmptyState
            emoji="🦗"
            title={t("thread.empty")}
            body={member ? (canPost ? t("thread.emptyBody") : undefined) : t("thread.emptyJoin")}
            action={
              member && canPost ? (
                <Button onClick={() => setComposing(true)}>{t("compose.new")}</Button>
              ) : undefined
            }
          />
        ) : (
          <>
            {pinned.map((thread) => (
              <ThreadCard
                key={`pin-${thread.id}`}
                thread={thread}
                handle={c.handle}
                channel={channel ? undefined : byId.get(thread.channelId)}
              />
            ))}
            {list.map((thread) => (
              <ThreadCard
                key={thread.id}
                thread={thread}
                handle={c.handle}
                channel={channel ? undefined : byId.get(thread.channelId)}
              />
            ))}
            <LoadMore query={threads} />
          </>
        )}
      </div>

      {channels.data && (
        <ThreadComposer
          community={c}
          channels={channels.data}
          channelId={channel?.id ?? ""}
          open={composing}
          onOpenChange={setComposing}
          onCreated={(thread) =>
            void navigate({
              to: "/g/$handle/t/$threadId",
              params: { handle: c.handle, threadId: thread.shortId || thread.id },
            })
          }
        />
      )}
    </div>
  );
}

function ChannelPill({
  active,
  onClick,
  label,
  emoji,
  unread,
  anonymous,
  readOnly,
}: {
  active: boolean;
  onClick(): void;
  label: string;
  emoji?: string;
  unread?: boolean;
  anonymous?: boolean;
  readOnly?: boolean;
}) {
  const { t } = useTranslation(NS);
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "relative inline-flex h-10 shrink-0 items-center gap-1.5 rounded-full border-2 px-3.5 text-sm font-bold transition-colors",
        active
          ? "border-ink bg-brand text-on-brand"
          : "border-line bg-surface hover:border-ink-faint",
      )}
    >
      {emoji && <span aria-hidden>{emoji}</span>}
      {label}
      {anonymous && <VenetianMask className="size-4" aria-label={t("anonymous.chip")} />}
      {readOnly && <Lock className="size-3.5" aria-label={t("channel.readOnly")} />}
      {unread && (
        <span
          role="img"
          aria-label={t("unread")}
          className="absolute -top-1 -right-1 size-3 rounded-full border-2 border-surface bg-pink"
        />
      )}
    </button>
  );
}
