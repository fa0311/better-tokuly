import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { stamp } from "@tokuly/api";
import { BadgeCheck, ChevronRight, Plus, ShieldCheck } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { formatCount, stampKeys, studioClient } from "./api";
import { PackFormDialog, type PackListing } from "./pack-form";
import { PackCover, StatusChip } from "./parts";

const FILTERS = ["all", "draft", "inReview", "published", "rejected"] as const;
type Filter = (typeof FILTERS)[number];
const FILTER_STATUS: Record<Filter, stamp.PackStatus> = {
  all: stamp.PackStatus.UNSPECIFIED,
  draft: stamp.PackStatus.DRAFT,
  inReview: stamp.PackStatus.IN_REVIEW,
  published: stamp.PackStatus.PUBLISHED,
  rejected: stamp.PackStatus.REJECTED,
};

type Scope = "mine" | "official";

/** The creator studio's front page: how the creator is doing, and their packs in every status. */
export function StudioScreen() {
  const { t } = useTranslation("stamps");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const admin = useMe()?.roles.includes("admin") ?? false;
  const [scope, setScope] = useState<Scope>("mine");
  const [filter, setFilter] = useState<Filter>("all");
  const [creating, setCreating] = useState(false);
  const official = admin && scope === "official";

  const profile = useQuery({ queryKey: stampKeys.creatorProfile, queryFn: async () => (await studioClient().getCreatorProfile({})).profile });
  const packs = useInfiniteQuery({
    queryKey: [...stampKeys.myPacks(official), filter],
    queryFn: ({ pageParam }) => studioClient().listMyPacks({ status: FILTER_STATUS[filter], official, page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const items = packs.data?.pages.flatMap((page) => page.packs) ?? [];

  const create = async (listing: PackListing) => {
    try {
      const { pack } = await studioClient().createPack({ ...listing, official });
      await queryClient.invalidateQueries({ queryKey: stampKeys.all });
      toast.success(t("toast.packCreated"));
      if (pack?.summary) await navigate({ to: "/stamps/studio/$packId", params: { packId: pack.summary.shortId || pack.summary.id } });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "stamps" }));
      throw error;
    }
  };

  const newPack = (
    <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
      {t("studio.newPack")}
    </Button>
  );

  return (
    <Page title={t("studio.title")} back="/stamps" wide>
      <div className="flex flex-col gap-5">
        <CreatorCard profile={profile.data} loading={profile.isPending} />

        <section aria-label={t("studio.packs")} className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <h2 className="text-lg font-black">{official ? t("studio.officialPacks") : t("studio.packs")}</h2>
            {newPack}
          </div>
          <div className="flex flex-wrap gap-2">
            {admin && (
              <Tabs<Scope>
                value={scope}
                onChange={setScope}
                items={[
                  { value: "mine", label: t("studio.scope.mine") },
                  { value: "official", label: t("studio.scope.official") },
                ]}
              />
            )}
            <Tabs<Filter> value={filter} onChange={setFilter} items={FILTERS.map((value) => ({ value, label: t(`studio.filters.${value}`) }))} />
          </div>

          {packs.isPending ? (
            <div className="grid gap-3 md:grid-cols-2">
              {[0, 1, 2, 3].map((index) => (
                <Skeleton key={index} className="h-24" />
              ))}
            </div>
          ) : items.length === 0 ? (
            filter === "all" ? (
              <EmptyState emoji="🎨" title={t("studio.empty")} body={t("studio.emptyBody")} action={newPack} />
            ) : (
              <EmptyState emoji="🫙" title={t("studio.emptyFilter")} />
            )
          ) : (
            <>
              <ul className="grid gap-3 md:grid-cols-2">
                {items.map((pack) => (
                  <li key={pack.id}>
                    <MyPackCard pack={pack} />
                  </li>
                ))}
              </ul>
              <LoadMore query={packs} />
            </>
          )}
        </section>
      </div>

      <PackFormDialog
        open={creating}
        onOpenChange={setCreating}
        title={official ? t("studio.newOfficialPack") : t("studio.newPack")}
        submitLabel={t("studio.create")}
        limits={profile.data}
        official={official}
        onSubmit={create}
      />
    </Page>
  );
}

function CreatorCard({ profile, loading }: { profile: stamp.CreatorProfile | undefined; loading: boolean }) {
  const { t } = useTranslation("stamps");
  if (loading || !profile) return <Skeleton className="h-40" />;
  return (
    <section aria-label={t("studio.profile")} className="sticker flex flex-col gap-4 rounded-card bg-surface p-4">
      <div className="flex items-center gap-3">
        <span className={`grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink text-[#17122b] ${profile.trusted ? "bg-lime" : "bg-sun"}`}>
          {profile.trusted ? <BadgeCheck className="size-6" /> : <ShieldCheck className="size-6" />}
        </span>
        <div className="min-w-0">
          <p className="font-black">{profile.trusted ? t("studio.trusted") : t("studio.reviewed")}</p>
          <p className="text-sm leading-snug text-ink-soft">{profile.trusted ? t("studio.trustedHint") : t("studio.reviewedHint")}</p>
        </div>
      </div>
      <dl className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
        <Figure label={t("studio.figures.packs")} value={`${profile.packCount} / ${profile.maxPacks}`} hint={t("studio.figures.published", { count: profile.publishedPackCount })} />
        <Figure label={t("stats.installs")} value={formatCount(profile.installCount)} />
        <Figure label={t("stats.sends")} value={formatCount(profile.sendCount)} />
        <Figure label={t("stats.sales")} value={formatCount(profile.saleCount)} />
        <Figure label={t("stats.earnings")} value={<Coins amount={profile.earnings} />} hint={t("studio.figures.fee", { fee: profile.feePercent })} />
      </dl>
      {profile.earnings > 0n && (
        <Link to="/wallet/earnings" className="inline-flex items-center self-start text-sm font-bold text-brand">
          {t("studio.toEarnings")}
          <ChevronRight className="size-4" />
        </Link>
      )}
    </section>
  );
}

function Figure({ label, value, hint }: { label: string; value: ReactNode; hint?: string }) {
  return (
    <div className="rounded-2xl bg-surface-2 px-3 py-2.5">
      <dt className="text-xs font-bold text-ink-soft">{label}</dt>
      <dd className="text-xl font-black">{value}</dd>
      {hint && <dd className="text-[11px] font-bold text-ink-faint">{hint}</dd>}
    </div>
  );
}

function MyPackCard({ pack }: { pack: stamp.PackSummary }) {
  const { t } = useTranslation("stamps");
  return (
    <Link
      to="/stamps/studio/$packId"
      params={{ packId: pack.shortId || pack.id }}
      className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
      data-testid="studio-pack"
    >
      <PackCover pack={pack} className="size-16 shrink-0 rounded-xl" />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{pack.name}</span>
        <span className="mt-1 flex flex-wrap items-center gap-1.5">
          <StatusChip status={pack.status} />
          <span className="text-xs font-bold text-ink-soft">{t("stampCount", { count: pack.stampCount })}</span>
          {pack.price > 0 && <Coins amount={pack.price} size="sm" />}
        </span>
        {pack.status === stamp.PackStatus.PUBLISHED && (
          <span className="mt-1 block text-xs text-ink-faint">{t("studio.packNumbers", { installs: formatCount(pack.installCount), sends: formatCount(pack.sendCount) })}</span>
        )}
      </span>
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </Link>
  );
}
