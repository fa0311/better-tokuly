import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, stamp, toDate } from "@tokuly/api";
import { BadgeCheck, Check, MoreHorizontal, ShieldCheck, ShieldOff, UserRound, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Coins } from "#/components/ui/coins";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { patchPack, reviewClient, stampKeys, storeClient } from "./api";
import { PackCover, PackRow, SearchField, StampTile } from "./parts";
import { StampImage } from "./stamp-image";

type Tab = "queue" | "store";

/**
 * The staff page for stamps, inside the admin console: the review queue
 * (approve or decline what creators submitted) and the store's shelves
 * (what is featured, which creators publish without review).
 */
export function ReviewScreen() {
  const { t } = useTranslation("stamps");
  const roles = useMe()?.roles ?? [];
  const moderator = roles.includes("admin") || roles.includes("moderator");
  const [tab, setTab] = useState<Tab>("queue");
  const queue = useInfiniteQuery({
    queryKey: stampKeys.reviewQueue,
    queryFn: ({ pageParam }) => reviewClient().listReviewQueue({ page: { limit: 10, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: moderator,
  });
  const waiting = queue.data?.pages[0]?.pendingCount;

  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("review.title")}</h1>
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">
        {!moderator ? (
          <EmptyState emoji="🔐" title={t("review.gateTitle")} body={t("review.gateBody")} />
        ) : (
          <>
            <Tabs<Tab>
              value={tab}
              onChange={setTab}
              items={[
                { value: "queue", label: t("review.tabs.queue"), badge: waiting },
                { value: "store", label: t("review.tabs.store") },
              ]}
            />
            <div className="mt-4">
              {tab === "queue" &&
                (queue.isPending ? (
                  <div className="flex flex-col gap-4">
                    <Skeleton className="h-64" />
                    <Skeleton className="h-64" />
                  </div>
                ) : queue.isError ? (
                  <EmptyState emoji="😵‍💫" title={t("store.error")} />
                ) : (
                  <Queue items={queue.data.pages.flatMap((page) => page.items)} waiting={waiting ?? 0} more={<LoadMore query={queue} />} />
                ))}
              {tab === "store" && <StoreShelves />}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ───────────────────────────── Queue ─────────────────────────────

function Queue({ items, waiting, more }: { items: stamp.ReviewItem[]; waiting: number; more: ReactNode }) {
  const { t } = useTranslation("stamps");
  if (items.length === 0) return <EmptyState emoji="🍵" title={t("review.empty")} body={t("review.emptyBody")} />;
  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm font-bold text-ink-soft">{t("review.waiting", { count: waiting })}</p>
      {items.map((item) => item.pack?.summary && <ReviewCard key={item.pack.summary.id} item={item} />)}
      {more}
    </div>
  );
}

function useTrust() {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { userId: string; trusted: boolean }) => reviewClient().setCreatorTrusted(input),
    onSuccess: (_, input) => {
      toast.success(input.trusted ? t("toast.trusted") : t("toast.untrusted"));
      return queryClient.invalidateQueries({ queryKey: stampKeys.review });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "stamps" })),
  });
}

/** One submission: everything a moderator needs to decide, and the two decisions. */
function ReviewCard({ item }: { item: stamp.ReviewItem }) {
  const { t, i18n } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const pack = item.pack!;
  const summary = pack.summary!;
  const creator = summary.creator;
  const submitted = toDate(pack.review?.submittedAt);
  const fresh = pack.stamps.filter((entry) => entry.pendingReview).length;
  // While a pack waits, "needs review" is exactly "its creator is not trusted".
  const trusted = pack.review ? !pack.review.needsReview : false;

  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [preview, setPreview] = useState<stamp.Stamp | null>(null);

  const decided = () => Promise.all([queryClient.invalidateQueries({ queryKey: stampKeys.review }), queryClient.invalidateQueries({ queryKey: stampKeys.store })]);
  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "stamps" }));
    // Someone else may have decided meanwhile: show the queue as it is.
    void queryClient.invalidateQueries({ queryKey: stampKeys.review });
  };
  const approve = useMutation({
    mutationFn: () => reviewClient().approvePack({ packId: summary.id }),
    onSuccess: () => {
      toast.success(t("toast.approved", { name: summary.name }));
      return decided();
    },
    onError,
  });
  const reject = useMutation({
    mutationFn: () => reviewClient().rejectPack({ packId: summary.id, reason: reason.trim() }),
    onSuccess: () => {
      setRejecting(false);
      toast.success(t("toast.rejected", { name: summary.name }));
      return decided();
    },
    onError,
  });
  const trust = useTrust();

  return (
    <article className="rounded-2xl border-2 border-line bg-surface" data-testid="review-item" aria-label={summary.name}>
      <header className="flex flex-wrap items-center gap-3 border-b-2 border-line p-4">
        <PackCover pack={summary} className="size-16 shrink-0 rounded-xl" />
        <div className="min-w-0 flex-1 basis-48">
          <h2 className="truncate text-lg font-black">{summary.name}</h2>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
            {summary.official ? (
              <span className="inline-flex items-center gap-1 font-bold text-brand">
                <BadgeCheck className="size-4" />
                {t("official")}
              </span>
            ) : creator ? (
              <Link to="/u/$handle" params={{ handle: creator.handle }} className="inline-flex min-w-0 items-center gap-1.5 font-bold hover:underline">
                <Avatar user={creator} size="xs" />
                <span className="truncate">{creator.displayName}</span>
                <span className="truncate text-ink-faint">@{creator.handle}</span>
              </Link>
            ) : (
              <span className="text-ink-faint">{t("deletedCreator")}</span>
            )}
            {summary.price > 0 ? <Coins amount={summary.price} size="sm" /> : <span className="font-bold text-ink-soft">{t("free")}</span>}
            {submitted && <span className="text-xs text-ink-faint">{t("review.submitted", { date: submitted.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" }) })}</span>}
          </div>
        </div>
        {/* On phones the two decisions get a row of their own, so the pack's details keep their room. */}
        <div className="flex w-full shrink-0 justify-end gap-2 sm:w-auto">
          <Button variant="secondary" icon={<X className="size-4" />} onClick={() => setRejecting(true)}>
            {t("review.reject")}
          </Button>
          <Button variant="lime" icon={<Check className="size-4" />} loading={approve.isPending} onClick={() => approve.mutate()}>
            {t("review.approve")}
          </Button>
        </div>
      </header>

      <div className="grid gap-4 p-4 lg:grid-cols-[1fr_20rem]">
        <div className="min-w-0">
          <p className="pb-1 text-xs font-bold text-ink-faint">{t("review.stamps", { count: pack.stamps.length, fresh })}</p>
          <div className="grid grid-cols-4 gap-1 rounded-2xl bg-surface-2 p-2 sm:grid-cols-6 xl:grid-cols-8">
            {pack.stamps.map((entry) => (
              <StampTile key={entry.stampRef?.id} item={entry} hidePending={fresh === pack.stamps.length} onPress={setPreview} />
            ))}
          </div>
        </div>
        <div className="flex min-w-0 flex-col gap-3 text-sm">
          {pack.description ? <p className="leading-relaxed break-words whitespace-pre-wrap">{pack.description}</p> : <p className="text-ink-faint">{t("studio.pack.noDescription")}</p>}
          {summary.tags.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {summary.tags.map((tag) => (
                <Chip key={tag}>#{tag}</Chip>
              ))}
            </div>
          )}
          {creator && !summary.official && (
            <div className="rounded-2xl border-2 border-line">
              <SwitchRow
                label={t("review.trust")}
                hint={t("review.trustHint")}
                checked={trusted}
                disabled={trust.isPending}
                onChange={(next) => trust.mutate({ userId: creator.id, trusted: next })}
              />
            </div>
          )}
          {item.history.length > 0 && (
            <div>
              <p className="pb-1 text-xs font-black text-ink-soft">{t("review.history")}</p>
              <ul className="flex flex-col gap-1.5">
                {item.history.map((record, index) => (
                  <li key={index} className="rounded-xl bg-surface-2 px-3 py-2">
                    <p className="flex flex-wrap items-baseline gap-x-2 text-xs font-bold">
                      <span>{t(decisionKey(record.decision))}</span>
                      <span className="text-ink-faint">
                        {toDate(record.decidedAt)?.toLocaleDateString(i18n.language)}
                        {record.moderator && ` · @${record.moderator.handle}`}
                      </span>
                    </p>
                    {record.note && <p className="mt-0.5 break-words whitespace-pre-wrap">{record.note}</p>}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </div>

      <ConfirmDialog
        open={rejecting}
        onOpenChange={setRejecting}
        title={t("review.rejectTitle", { name: summary.name })}
        description={t("review.rejectBody")}
        confirmLabel={t("review.reject")}
        danger
        loading={reject.isPending}
        onConfirm={() => (reason.trim() ? reject.mutate() : toast.error(t("review.reasonRequired")))}
      >
        <TextArea label={t("review.reason")} placeholder={t("review.reasonPlaceholder")} value={reason} maxLength={500} rows={3} autoFocus onChange={(event) => setReason(event.target.value)} />
      </ConfirmDialog>
      <Dialog open={preview !== null} onOpenChange={(open) => !open && setPreview(null)} title={preview?.stampRef?.name || t("stamp.unnamed")}>
        {preview?.stampRef && (
          <div className="flex flex-col items-center gap-3">
            <div className="grid w-full place-items-center rounded-[2rem] bg-surface-2 py-6">
              <StampImage stamp={preview.stampRef} size={224} play="always" />
            </div>
            <div className="flex flex-wrap justify-center gap-1.5">
              {preview.pendingReview && <Chip className="bg-sun text-[#17122b]">{t("stamp.pending")}</Chip>}
              {preview.emoji && <Chip className="text-sm">{preview.emoji}</Chip>}
              {preview.keywords.map((keyword) => (
                <Chip key={keyword}>{keyword}</Chip>
              ))}
            </div>
          </div>
        )}
      </Dialog>
    </article>
  );
}

function decisionKey(decision: stamp.ReviewDecision): string {
  switch (decision) {
    case stamp.ReviewDecision.APPROVED:
      return "review.decisions.approved";
    case stamp.ReviewDecision.REJECTED:
      return "review.decisions.rejected";
    case stamp.ReviewDecision.REMOVED:
      return "review.decisions.removed";
    default:
      return "review.decisions.unknown";
  }
}

// ───────────────────────────── Store shelves ─────────────────────────────

const SHELVES = ["featured", "new", "popular"] as const;
type Shelf = (typeof SHELVES)[number];
const SHELF_SECTION: Record<Shelf, stamp.StoreSection> = {
  featured: stamp.StoreSection.FEATURED,
  new: stamp.StoreSection.NEW,
  popular: stamp.StoreSection.POPULAR,
};

/** Published packs with the two things moderation decides about them: featured or not, creator trusted or not. */
function StoreShelves() {
  const { t } = useTranslation("stamps");
  const [shelf, setShelf] = useState<Shelf>("featured");
  const [text, setText] = useState("");
  const [query, setQuery] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setQuery(text.trim()), 300);
    return () => clearTimeout(timer);
  }, [text]);

  const packs = useInfiniteQuery({
    queryKey: query ? stampKeys.storeSearch(query, stamp.PriceFilter.UNSPECIFIED) : stampKeys.storeList(SHELF_SECTION[shelf], "", stamp.PriceFilter.UNSPECIFIED),
    queryFn: async ({ pageParam }) => {
      const page = { limit: 24, cursor: pageParam };
      const response = query ? await storeClient().searchPacks({ query, page }) : await storeClient().listStorePacks({ section: SHELF_SECTION[shelf], page });
      return { packs: response.packs, next: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.next || undefined,
  });
  const items = packs.data?.pages.flatMap((page) => page.packs) ?? [];

  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm leading-relaxed text-ink-soft">{t("review.storeLead")}</p>
      <div className="flex flex-wrap items-center gap-2">
        <SearchField className="min-w-56 flex-1" value={text} onChange={setText} placeholder={t("review.storeSearch")} />
        {!query && <Tabs<Shelf> value={shelf} onChange={setShelf} items={SHELVES.map((value) => ({ value, label: t(`review.shelves.${value}`) }))} />}
      </div>
      {packs.isPending ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {[0, 1, 2, 3].map((index) => (
            <Skeleton key={index} className="h-32" />
          ))}
        </div>
      ) : items.length === 0 ? (
        <EmptyState emoji="🧺" title={query ? t("browse.noResults", { query }) : shelf === "featured" ? t("review.noFeatured") : t("browse.empty")} />
      ) : (
        <>
          <ul className="grid gap-3 lg:grid-cols-2 2xl:grid-cols-3">
            {items.map((pack) => (
              <li key={pack.id}>
                <ShelfCard pack={pack} />
              </li>
            ))}
          </ul>
          <LoadMore query={packs} />
        </>
      )}
    </div>
  );
}

function ShelfCard({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation("stamps");
  const queryClient = useQueryClient();
  const feature = useMutation({
    mutationFn: (featured: boolean) => reviewClient().setPackFeatured({ packId: pack.id, featured }),
    // The switch flips at once and flips back if the server refuses.
    onMutate: (featured) => patchPack(queryClient, pack.id, (summary) => ({ ...summary, featured })),
    onSuccess: (response, featured) => {
      const updated = response.pack;
      if (updated) patchPack(queryClient, pack.id, (summary) => ({ ...summary, featured: updated.featured }));
      toast.success(featured ? t("toast.featured", { name: pack.name }) : t("toast.unfeatured", { name: pack.name }));
      return queryClient.invalidateQueries({ queryKey: stampKeys.store });
    },
    onError: (error, featured) => {
      patchPack(queryClient, pack.id, (summary) => ({ ...summary, featured: !featured }));
      toast.error(errorMessage(error, { namespace: "stamps" }));
    },
  });

  return (
    <div className="rounded-2xl border-2 border-line bg-surface p-1" data-testid="shelf-pack">
      <PackRow pack={pack} trailing={pack.creator && !pack.official ? <CreatorMenu creator={pack.creator} /> : undefined} />
      <SwitchRow label={t("review.featured")} hint={t("review.featuredHint")} checked={pack.featured} disabled={feature.isPending} onChange={(next) => feature.mutate(next)} />
    </div>
  );
}

/**
 * Trust controls for the creator of a published pack. The API does not say
 * whether a creator is trusted right now, so both directions are offered
 * (and confirmed) instead of a switch that would have to guess its state.
 */
function CreatorMenu({ creator }: { creator: common.UserSummary }) {
  const { t } = useTranslation("stamps");
  const trust = useTrust();
  const [asking, setAsking] = useState<boolean | null>(null);
  return (
    <>
      <Menu
        trigger={
          <Button size="icon-sm" variant="ghost" aria-label={t("review.creatorMenu", { name: creator.displayName })}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={[
          { label: t("review.trustCreator"), icon: <ShieldCheck />, onSelect: () => setAsking(true) },
          { label: t("review.untrustCreator"), icon: <ShieldOff />, onSelect: () => setAsking(false) },
          { label: t("review.openProfile"), icon: <UserRound />, onSelect: () => window.open(`/u/${creator.handle}`, "_blank", "noopener") },
        ]}
      />
      <ConfirmDialog
        open={asking !== null}
        onOpenChange={(open) => !open && setAsking(null)}
        title={asking ? t("review.trustTitle", { name: creator.displayName }) : t("review.untrustTitle", { name: creator.displayName })}
        description={asking ? t("review.trustHint") : t("review.untrustHint")}
        confirmLabel={asking ? t("review.trustCreator") : t("review.untrustCreator")}
        loading={trust.isPending}
        onConfirm={() => asking !== null && trust.mutate({ userId: creator.id, trusted: asking }, { onSettled: () => setAsking(null) })}
      />
    </>
  );
}
