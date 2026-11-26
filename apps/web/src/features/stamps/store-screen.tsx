import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import type { stamp } from "@tokuly/api";
import { stampQueries } from "@tokuly/core/stamps";
import { BadgeCheck, ChevronRight, LibraryBig, Palette } from "lucide-react";
import { type ComponentType, useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { BalancePill } from "#/features/wallet";
import { cn } from "#/lib/cn";
import { storeClient } from "./api";
import { PackCard, PackRow, SearchField, SectionHeading, TagLink } from "./parts";

type BrowseSection = "featured" | "popular" | "new" | "official";

/** The stamp store's front page: search, shortcuts, and the featured, popular and new shelves. */
export function StoreScreen() {
  const { t } = useTranslation("stamps");
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const { data, isPending, isError } = useQuery(stampQueries.storeHome(storeClient));
  const empty = data && data.featured.length + data.popular.length + data.newest.length === 0;

  return (
    <Page title={t("title")} wide actions={<BalancePill />}>
      <div className="flex flex-col gap-6">
        <div className="flex flex-col gap-3 md:flex-row md:items-center">
          <SearchField
            className="md:flex-1"
            value={query}
            onChange={setQuery}
            placeholder={t("search.placeholder")}
            onSubmit={() =>
              query.trim() && navigate({ to: "/stamps/browse", search: { q: query.trim() } })
            }
          />
          <div className="grid grid-cols-2 gap-3 md:w-[26rem]">
            <Shortcut
              to="/stamps/library"
              icon={LibraryBig}
              tint="bg-sky"
              label={t("shortcuts.library")}
              hint={t("shortcuts.libraryHint")}
            />
            <Shortcut
              to="/stamps/studio"
              icon={Palette}
              tint="bg-pink"
              label={t("shortcuts.studio")}
              hint={t("shortcuts.studioHint")}
            />
          </div>
        </div>

        {isPending ? (
          <StoreSkeleton />
        ) : isError || empty || !data ? (
          <EmptyState
            emoji="🧸"
            title={isError ? t("store.error") : t("store.empty")}
            body={isError ? undefined : t("store.emptyBody")}
          />
        ) : (
          <>
            {data.tags.length > 0 && (
              <div className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1 md:mx-0 md:flex-wrap md:px-0">
                {data.tags.slice(0, 16).map((tag) => (
                  <TagLink key={tag.tag} tag={tag.tag} count={tag.packCount} />
                ))}
              </div>
            )}
            <Shelf title={t("store.featured")} section="featured" packs={data.featured} />
            <Ranking packs={data.popular} />
            <Shelf title={t("store.new")} section="new" packs={data.newest} />
            <Link
              to="/stamps/browse"
              search={{ section: "official" }}
              className="pressable sticker flex items-center gap-3 rounded-card bg-brand-soft p-4"
            >
              <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-brand text-on-brand">
                <BadgeCheck className="size-6" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block font-black">{t("store.official")}</span>
                <span className="block truncate text-sm text-ink-soft">
                  {t("store.officialHint")}
                </span>
              </span>
              <ChevronRight className="size-5 shrink-0 text-ink-faint" />
            </Link>
          </>
        )}
      </div>
    </Page>
  );
}

function Shortcut({
  to,
  icon: Icon,
  tint,
  label,
  hint,
}: {
  to: "/stamps/library" | "/stamps/studio";
  icon: ComponentType<{ className?: string }>;
  tint: string;
  label: string;
  hint: string;
}) {
  return (
    <Link
      to={to}
      className="pressable sticker flex items-center gap-2.5 rounded-card bg-surface p-2.5"
    >
      <span
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-2xl border-2 border-ink text-[#17122b]",
          tint,
        )}
      >
        <Icon className="size-5" />
      </span>
      <span className="min-w-0">
        <span className="block truncate font-black leading-tight">{label}</span>
        <span className="block truncate text-xs text-ink-soft">{hint}</span>
      </span>
    </Link>
  );
}

function MoreLink({ section }: { section: BrowseSection }) {
  const { t } = useTranslation("stamps");
  return (
    <Link
      to="/stamps/browse"
      search={{ section }}
      className="inline-flex items-center text-sm font-bold text-brand"
    >
      {t("store.more")}
      <ChevronRight className="size-4" />
    </Link>
  );
}

/** A row of pack cards: swiped sideways on phones, a grid on wide screens. */
function Shelf({
  title,
  section,
  packs,
}: {
  title: string;
  section: BrowseSection;
  packs: stamp.PackSummary[];
}) {
  if (packs.length === 0) return null;
  return (
    <section aria-label={title}>
      <SectionHeading title={title} action={<MoreLink section={section} />} />
      <div className="scrollbar-none -mx-4 flex snap-x gap-3 overflow-x-auto px-4 pt-1 pb-2 md:mx-0 md:grid md:grid-cols-4 md:overflow-visible md:px-0 lg:grid-cols-6">
        {packs.map((pack) => (
          <PackCard key={pack.id} pack={pack} className="w-36 shrink-0 snap-start md:w-auto" />
        ))}
      </div>
    </section>
  );
}

/** The popular shelf as a ranking: first place first. */
function Ranking({ packs }: { packs: stamp.PackSummary[] }) {
  const { t } = useTranslation("stamps");
  if (packs.length === 0) return null;
  return (
    <section aria-label={t("store.popular")}>
      <SectionHeading title={t("store.popular")} action={<MoreLink section="popular" />} />
      <div className="sticker grid rounded-card bg-surface p-2 md:grid-cols-2 lg:grid-cols-3">
        {packs.slice(0, 9).map((pack, index) => (
          <PackRow key={pack.id} pack={pack} rank={index + 1} />
        ))}
      </div>
    </section>
  );
}

function StoreSkeleton() {
  return (
    <div className="flex flex-col gap-6">
      <div className="flex gap-2">
        {[0, 1, 2, 3].map((index) => (
          <Skeleton key={index} className="h-9 w-24 rounded-full" />
        ))}
      </div>
      {[0, 1].map((row) => (
        <div key={row}>
          <Skeleton className="mb-3 h-6 w-32" />
          <div className="flex gap-3 overflow-hidden md:grid md:grid-cols-4 lg:grid-cols-6">
            {[0, 1, 2, 3, 4, 5].map((index) => (
              <Skeleton key={index} className="aspect-[4/5] w-36 shrink-0 md:w-auto" />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
