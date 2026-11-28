import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { stamp } from "@tokuly/api";
import { stampQueries } from "@tokuly/core/stamps";
import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useTranslation } from "#/lib/use-translation";
import { storeClient } from "./api";
import { type BrowseSearch, PRICES, type Price, SECTIONS, type Section } from "./browse-search";
import { PackCard, PackGridSkeleton, SearchField, TagLink } from "./parts";

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

  const update = (patch: BrowseSearch) =>
    navigate({ to: "/stamps/browse", search: { ...search, ...patch }, replace: true });

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
    const timer = setTimeout(
      () =>
        void navigate({
          to: "/stamps/browse",
          search: (current) => ({ ...current, q: typed || undefined }),
          replace: true,
        }),
      300,
    );
    return () => clearTimeout(timer);
  }, [text, query, navigate]);

  const packs = useInfiniteQuery(
    stampQueries.shelf(
      storeClient,
      { section: SECTION_VALUE[section], tag, price: PRICE_VALUE[price], query },
      PAGE_SIZE,
    ),
  );
  const items = packs.data?.pages.flatMap((page) => page.packs) ?? [];
  // Tags to narrow a shelf by, offered until one is chosen.
  const tags = useQuery({ ...stampQueries.tags(storeClient), enabled: !query && !tag });

  return (
    <Page
      title={query ? t("browse.searchTitle") : tag ? `#${tag}` : t(`browse.sections.${section}`)}
      back="/stamps"
      wide
    >
      <div className="flex flex-col gap-3">
        <SearchField
          value={text}
          onChange={setText}
          placeholder={t("search.placeholder")}
          autoFocus={Boolean(query)}
        />
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
        {!query && !tag && tags.data && tags.data.tags.length > 0 && (
          <nav
            aria-label={t("browse.tags")}
            className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1 md:mx-0 md:flex-wrap md:px-0"
          >
            {tags.data.tags.map((item) => (
              <TagLink key={item.tag} tag={item.tag} count={item.packCount} />
            ))}
          </nav>
        )}

        {packs.isPending ? (
          <PackGridSkeleton count={12} />
        ) : packs.isError ? (
          <EmptyState emoji="😵‍💫" title={t("store.error")} />
        ) : items.length === 0 ? (
          <EmptyState
            emoji={query ? "🔍" : "🧺"}
            title={query ? t("browse.noResults", { query }) : t("browse.empty")}
            body={query ? t("browse.noResultsBody") : undefined}
          />
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
