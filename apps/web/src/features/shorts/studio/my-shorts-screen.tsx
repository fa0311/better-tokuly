import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { shorts } from "@tokuly/api";
import { imageUrl } from "@tokuly/core/shorts";
import {
  BarChart3,
  Eye,
  Heart,
  MessageCircle,
  MoreVertical,
  Pencil,
  Pin,
  PinOff,
  Play,
  Plus,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { EmojiText } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { useMyChannels } from "#/features/video";
import { cn } from "#/lib/cn";
import { errorMessage, reasonText } from "#/lib/errors";
import { waitForAsset } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import {
  nextCursor,
  patchShortEverywhere,
  removeShortEverywhere,
  shortClient,
  shortKeys,
} from "../api";
import { ChoiceChips } from "../sort-chips";
import { visibilityKey } from "./settings-fields";

const { ShortStatus } = shorts;

const STATUS_KEYS: Partial<Record<shorts.ShortStatus, string>> = {
  [ShortStatus.PROCESSING]: "processing",
  [ShortStatus.PUBLISHED]: "published",
  [ShortStatus.FAILED]: "failed",
};

/**
 * A short that is still being processed goes live by itself. The media
 * service says when its upload is done; the short follows a moment later, so
 * it is read again until it has moved on (a few tries, then the next visit).
 */
export function useProcessingWatch(short: shorts.Short | undefined): void {
  const queryClient = useQueryClient();
  const id = short?.id ?? "";
  const assetId = short?.status === ShortStatus.PROCESSING ? (short.studio?.assetId ?? "") : "";
  useEffect(() => {
    if (!id || !assetId) return;
    const controller = new AbortController();
    const refresh = async () => {
      for (let attempt = 0; attempt < 5 && !controller.signal.aborted; attempt++) {
        const current = (await shortClient().getShort({ shortId: id })).short;
        if (current && current.status !== ShortStatus.PROCESSING) {
          patchShortEverywhere(queryClient, id, () => current);
          void queryClient.invalidateQueries({ queryKey: shortKeys.lists });
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 1500));
      }
    };
    waitForAsset(assetId, undefined, controller.signal)
      .catch(() => {})
      .then(refresh)
      .catch(() => {});
    return () => controller.abort();
  }, [queryClient, id, assetId]);
}

/** `/shorts/studio`: everything the creator posted, in every state, with the way to each one's editor and numbers. */
export function MyShortsScreen() {
  const { t } = useTranslation("shorts");
  const channelsQuery = useMyChannels();
  const channels = channelsQuery.data?.channels ?? [];
  const [channelId, setChannelId] = useState("");
  const [status, setStatus] = useState<shorts.ShortStatus>(ShortStatus.UNSPECIFIED);
  const query = useInfiniteQuery({
    queryKey: shortKeys.mine(channelId, status),
    queryFn: ({ pageParam }) =>
      shortClient().listMyShorts({ channelId, status, page: { limit: 20, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: nextCursor,
    enabled: channels.length > 0,
  });
  const list = query.data?.pages.flatMap((page) => page.shorts) ?? [];

  const actions = (
    <>
      <Link
        to="/shorts/studio/analytics"
        aria-label={t("studio.analytics")}
        className="grid size-10 place-items-center rounded-full hover:bg-surface-2"
      >
        <BarChart3 className="size-5" />
      </Link>
      <Link
        to="/shorts/new"
        className="pressable sticker-sm inline-flex h-10 items-center gap-1.5 rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
      >
        <Plus className="size-4" />
        {t("studio.post")}
      </Link>
    </>
  );

  return (
    <Page title={t("studio.title")} back="/shorts" actions={actions}>
      {channelsQuery.isPending ? (
        <Skeleton className="h-40 rounded-card" />
      ) : channels.length === 0 ? (
        <EmptyState
          emoji="📡"
          title={t("create.needChannel")}
          body={t("create.needChannelBody")}
          action={
            <Link
              to="/studio/channels/new"
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              <Plus className="size-4" />
              {t("create.makeChannel")}
            </Link>
          }
        />
      ) : (
        <>
          <div className="mb-4 flex flex-col gap-3">
            {channels.length > 1 && (
              <SelectField
                label={t("form.channel")}
                value={channelId || "all"}
                onChange={(value) => setChannelId(value === "all" ? "" : value)}
                options={[
                  { value: "all", label: t("studio.allChannels") },
                  ...channels.map((entry) => ({
                    value: entry.summary?.id ?? "",
                    label: entry.summary?.name ?? "",
                  })),
                ]}
              />
            )}
            <ChoiceChips
              label={t("studio.filterLabel")}
              value={status}
              onChange={(next) => setStatus(next)}
              options={[
                { value: ShortStatus.UNSPECIFIED, label: t("studio.filter.all") },
                { value: ShortStatus.PUBLISHED, label: t("studio.filter.published") },
                { value: ShortStatus.PROCESSING, label: t("studio.filter.processing") },
                { value: ShortStatus.FAILED, label: t("studio.filter.failed") },
              ]}
            />
          </div>
          {query.isPending ? (
            <div className="flex flex-col gap-3">
              {Array.from({ length: 3 }, (_, index) => (
                <Skeleton key={index} className="h-28 rounded-card" />
              ))}
            </div>
          ) : query.isError ? (
            <EmptyState emoji="😵‍💫" title={errorMessage(query.error, { namespace: "shorts" })} />
          ) : list.length === 0 ? (
            <EmptyState
              emoji="⚡"
              title={
                status === ShortStatus.UNSPECIFIED ? t("studio.empty") : t("studio.emptyFilter")
              }
              body={status === ShortStatus.UNSPECIFIED ? t("studio.emptyBody") : undefined}
            />
          ) : (
            <>
              <ul className="flex flex-col gap-3">
                {list.map((short) => (
                  <li key={short.id}>
                    <MyShortRow short={short} />
                  </li>
                ))}
              </ul>
              <LoadMore query={query} />
            </>
          )}
        </>
      )}
    </Page>
  );
}

function MyShortRow({ short }: { short: shorts.Short }) {
  const { t, i18n } = useTranslation("shorts");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [deleting, setDeleting] = useState(false);
  useProcessingWatch(short);
  const published = short.status === ShortStatus.PUBLISHED;
  const cover = imageUrl(short.cover, 80);
  const count = (value: bigint) => compactCount(Number(value), i18n.language);

  const pin = useMutation({
    mutationFn: async (pinned: boolean) =>
      (
        await (pinned
          ? shortClient().pinShort({ shortId: short.id })
          : shortClient().unpinShort({ shortId: short.id }))
      ).short,
    onSuccess: (updated, pinned) => {
      if (updated) patchShortEverywhere(queryClient, short.id, () => updated);
      void queryClient.invalidateQueries({ queryKey: shortKeys.lists });
      toast.success(pinned ? t("studio.pinned") : t("studio.unpinned"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "shorts" })),
  });
  const remove = useMutation({
    mutationFn: () => shortClient().deleteShort({ shortId: short.id }),
    onSuccess: () => {
      setDeleting(false);
      removeShortEverywhere(queryClient, short.id);
      toast.success(t("studio.deleted"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "shorts" })),
  });

  const statusKey = STATUS_KEYS[short.status] ?? "processing";
  return (
    <article
      className="sticker flex gap-3 rounded-card bg-surface p-3"
      data-testid="my-short"
      data-status={statusKey}
    >
      <Link
        to="/shorts/studio/$shortId"
        params={{ shortId: short.shortId }}
        aria-label={t("studio.edit")}
        className="relative block aspect-[9/16] w-20 shrink-0 overflow-hidden rounded-2xl border-2 border-ink bg-surface-3"
        style={short.cover?.dominantColor ? { background: short.cover.dominantColor } : undefined}
      >
        {cover && <img src={cover} alt="" className="size-full object-cover" />}
      </Link>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <p className="line-clamp-2 font-bold leading-snug">
          {short.caption ? (
            <EmojiText text={short.caption} />
          ) : (
            <span className="text-ink-faint">{t("studio.noCaption")}</span>
          )}
        </p>
        <div className="flex flex-wrap items-center gap-1.5 text-xs font-black">
          <span
            className={cn(
              "rounded-full border-2 border-ink px-2 py-0.5",
              short.status === ShortStatus.PUBLISHED && "bg-lime text-[#17122b]",
              short.status === ShortStatus.PROCESSING && "bg-sun text-[#17122b]",
              short.status === ShortStatus.FAILED && "bg-danger text-white",
            )}
            data-testid="my-short-status"
          >
            {t(`status.${statusKey}`)}
          </span>
          <span className="rounded-full bg-surface-2 px-2 py-0.5 text-ink-soft">
            {t(`visibility.${visibilityKey(short.visibility)}.label`)}
          </span>
          {short.pinned && (
            <span className="flex items-center gap-1 rounded-full bg-surface-2 px-2 py-0.5 text-ink-soft">
              <Pin className="size-3" aria-hidden />
              {t("tile.pinned")}
            </span>
          )}
        </div>
        {short.status === ShortStatus.FAILED && (
          <p className="text-sm text-danger">
            {reasonText(short.studio?.processingError ?? "", { namespace: "shorts" }) ??
              t("errors:PROCESSING_FAILED")}
          </p>
        )}
        {published && (
          <p className="mt-auto flex flex-wrap gap-x-3 gap-y-1 text-sm font-bold text-ink-soft">
            <span className="flex items-center gap-1" title={t("analytics.metric.views")}>
              <Eye className="size-4" aria-hidden />
              <span data-testid="my-short-views">{count(short.viewCount)}</span>
            </span>
            <span className="flex items-center gap-1" title={t("analytics.metric.likes")}>
              <Heart className="size-4" aria-hidden />
              {count(short.likeCount)}
            </span>
            <span className="flex items-center gap-1" title={t("analytics.metric.comments")}>
              <MessageCircle className="size-4" aria-hidden />
              {count(short.commentCount)}
            </span>
          </p>
        )}
      </div>
      <Menu
        trigger={
          <button
            type="button"
            aria-label={t("studio.rowMore")}
            className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
          >
            <MoreVertical className="size-5" />
          </button>
        }
        actions={[
          {
            label: t("studio.edit"),
            icon: <Pencil />,
            onSelect: () =>
              void navigate({ to: "/shorts/studio/$shortId", params: { shortId: short.shortId } }),
          },
          ...(published
            ? [
                {
                  label: t("studio.watch"),
                  icon: <Play />,
                  onSelect: () =>
                    void navigate({ to: "/shorts/$shortId", params: { shortId: short.shortId } }),
                },
                {
                  label: t("studio.numbers"),
                  icon: <BarChart3 />,
                  onSelect: () =>
                    void navigate({
                      to: "/shorts/studio/analytics",
                      search: { short: short.shortId },
                    }),
                },
                short.pinned
                  ? {
                      label: t("studio.unpin"),
                      icon: <PinOff />,
                      onSelect: () => pin.mutate(false),
                    }
                  : { label: t("studio.pin"), icon: <Pin />, onSelect: () => pin.mutate(true) },
              ]
            : []),
          {
            label: t("studio.delete"),
            icon: <Trash2 />,
            danger: true,
            onSelect: () => setDeleting(true),
          },
        ]}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("studio.deleteTitle")}
        description={t("studio.deleteBody")}
        confirmLabel={t("studio.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </article>
  );
}
