import { useInfiniteQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { stamp } from "@tokuly/api";
import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { stampKeys, storeClient } from "./api";
import { PackCard, PackGridSkeleton, SearchField } from "./parts";

const SECTIONS = ["popular", "new", "featured", "official"] as const;
const PRICES = ["all", "free", "paid"] as const;
type Section = (typeof SECTIONS)[number];
type Price = (typeof PRICES)[number];

/** What the browse page shows, as kept in the address: a search, or a shelf with filters. */
export interface BrowseSearch {
  q?: string;
  section?: Section;
  tag?: string;
  price?: Exclude<Price, "all">;
}

/** Reads the browse page's address parameters, ignoring anything unknown. */
export function validateBrowseSearch(search: Record<string, unknown>): BrowseSearch {
  const text = (value: unknown) => (typeof value === "string" && value.trim() ? value.trim().slice(0, 64) : undefined);
  return {
    q: text(search.q),
    section: SECTIONS.find((section) => section === search.section),
    tag: text(search.tag)?.replace(/^#+/, "").toLowerCase() || undefined,
    price: search.price === "free" || search.price === "paid" ? search.price : undefined,
  };
}

const SECTION_VALUE: Record<Section, stamp.StoreSection> = {
  popular: stamp.StoreSection.POPULAR,
  new: stamp.StoreSection.NEW,
  featured: stamp.StoreSection.FEATURED,
  official: stamp.StoreSection.OFFICIAL,
};
const PRICE_VALUE: Record<Price, stamp.PriceFilter> = {
  all: stamp.PriceFilter.UNSPECIFIED,
  free: stamp.PriceFilter.FREE,
  paid: stamp.PriceFilter.PAID,
};

const PAGE_SIZE = 24;

/** Every pack of a shelf or a tag with paging, and the store search. */
export function BrowseScreen({ search }: { search: BrowseSearch }) {
  const { t } = useTranslation("stamps");
  const navigate = useNavigate();
  const section = search.section ?? "popular";
  const price = search.price ?? "all";
  const tag = search.tag ?? "";
  const query = search.q ?? "";

  const update = (patch: BrowseSearch) => navigate({ to: "/stamps/browse", search: { ...search, ...patch }, replace: true });

  // The box follows typing at once; the address (and with it the search) a moment later.
  const [text, setText] = useState(query);
  // When the address changes on its own (back, a link), the box follows it.
  const [followed, setFollowed] = useState(query);
  if (query !== followed) {
    setFollowed(query);
    if (text.trim() !== query) setText(query);
  }
  useEffect(() => {
    const typed = text.trim();
    if (typed === query) return;
    const timer = setTimeout(() => void navigate({ to: "/stamps/browse", search: (current) => ({ ...current, q: typed || undefined }), replace: true }), 300);
    return () => clearTimeout(timer);
  }, [text, query, navigate]);

  const packs = useInfiniteQuery({
    queryKey: query ? stampKeys.storeSearch(query, PRICE_VALUE[price]) : stampKeys.storeList(SECTION_VALUE[section], tag, PRICE_VALUE[price]),
    queryFn: ({ pageParam }) => {
      const page = { limit: PAGE_SIZE, cursor: pageParam };
      return query
        ? storeClient().searchPacks({ query, price: PRICE_VALUE[price], page })
        : storeClient().listStorePacks({ section: SECTION_VALUE[section], tag, price: PRICE_VALUE[price], page });
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const items = packs.data?.pages.flatMap((page) => page.packs) ?? [];

  return (
    <Page title={query ? t("browse.searchTitle") : tag ? `#${tag}` : t(`browse.sections.${section}`)} back="/stamps" wide>
      <div className="flex flex-col gap-3">
        <SearchField value={text} onChange={setText} placeholder={t("search.placeholder")} autoFocus={Boolean(query)} />
        <div className="flex flex-wrap items-center gap-2">
          {!query && (
            <Tabs<Section>
              value={section}
              onChange={(next) => update({ section: next })}
              items={SECTIONS.map((value) => ({ value, label: t(`browse.sections.${value}`) }))}
            />
          )}
          <Tabs<Price>
            value={price}
            onChange={(next) => update({ price: next === "all" ? undefined : next })}
            items={PRICES.map((value) => ({ value, label: t(`browse.prices.${value}`) }))}
          />
          {tag && !query && (
            <button
              type="button"
              onClick={() => update({ tag: undefined })}
              aria-label={t("browse.clearTag", { tag })}
              className="pressable sticker-sm inline-flex h-9 items-center gap-1 rounded-full bg-brand px-3 text-sm font-bold text-on-brand"
            >
              #{tag}
              <X className="size-4" />
            </button>
          )}
        </div>

        {packs.isPending ? (
          <PackGridSkeleton count={12} />
        ) : packs.isError ? (
          <EmptyState emoji="😵‍💫" title={t("store.error")} />
        ) : items.length === 0 ? (
          <EmptyState emoji={query ? "🔍" : "🧺"} title={query ? t("browse.noResults", { query }) : t("browse.empty")} body={query ? t("browse.noResultsBody") : undefined} />
        ) : (
          <>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6">
              {items.map((pack) => (
                <PackCard key={pack.id} pack={pack} />
              ))}
            </div>
            <LoadMore query={packs} />
          </>
        )}
      </div>
    </Page>
  );
}
