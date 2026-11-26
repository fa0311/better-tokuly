import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type CatalogEmoji, emojiKeys, toCatalogEmoji } from "@tokuly/core/emoji";
import { profileKeys } from "@tokuly/core/profile";
import { ClipboardList, Plus, Star, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { profileClient } from "#/lib/api";
import { emojiClient } from "./api";
import { useCatalog } from "./catalog";
import {
  EmojiTile,
  FilterPill,
  SearchBox,
  TileGrid,
  TileSkeletons,
  useGrowingCount,
} from "./parts";
import { setShowSensitive, useShowSensitive } from "./prefs";

const SORTS = ["all", "popular", "new"] as const;
type Sort = (typeof SORTS)[number];

/** What the catalogue page shows, as kept in the address: `/emoji?q=ねこ&category=どうぶつ&sort=popular`. */
export interface BrowseSearch {
  q?: string;
  /** A category's name; the empty string means "emoji without a category". */
  category?: string;
  sort?: Sort;
  /** Only the emoji credited to this user (the link on a profile's "emoji made" card). */
  author?: string;
}

export function validateBrowseSearch(search: Record<string, unknown>): BrowseSearch {
  const text = (value: unknown) =>
    typeof value === "string"
      ? value.trim().slice(0, 64)
      : typeof value === "number"
        ? String(value)
        : undefined;
  return {
    q: text(search.q) || undefined,
    category: text(search.category),
    sort: SORTS.find((sort) => sort === search.sort),
    author: text(search.author) || undefined,
  };
}

/** The catalogue: every approved emoji, to browse by category, popularity and age, and to search. */
export function BrowseScreen({ search }: { search: BrowseSearch }) {
  const { t } = useTranslation("emoji");
  const navigate = useNavigate();
  const catalog = useCatalog();
  const showSensitive = useShowSensitive();
  const sort = search.sort ?? "all";
  const query = search.q ?? "";
  const { category, author } = search;
  const update = (patch: BrowseSearch) =>
    navigate({ to: "/emoji", search: { ...search, ...patch }, replace: true });

  // The box follows the typing at once; the address (and with it the search) a moment later.
  const [typed, setTyped] = useState(query);
  useEffect(() => {
    if (typed.trim() === query) return;
    const timer = setTimeout(
      () =>
        void navigate({
          to: "/emoji",
          search: (current) => ({ ...validateBrowseSearch(current), q: typed.trim() || undefined }),
          replace: true,
        }),
      250,
    );
    return () => clearTimeout(timer);
  }, [typed, query, navigate]);

  const inCategory = (entry: CatalogEmoji) => category === undefined || entry.category === category;
  const uncategorized = catalog.emojis.some((entry) => !entry.category);

  return (
    <Page
      title={t("title")}
      back="/me"
      wide
      actions={
        <>
          <HeaderLink to="/emoji/my-deck" label={t("deck.title")}>
            <Star className="size-5" />
          </HeaderLink>
          <HeaderLink to="/emoji/submissions" label={t("submissions.title")}>
            <ClipboardList className="size-5" />
          </HeaderLink>
          <Link
            to="/emoji/new-emoji"
            className="pressable sticker inline-flex h-9 items-center gap-1.5 rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
          >
            <Plus className="size-4" />
            {t("submit.action")}
          </Link>
        </>
      }
    >
      {author ? (
        <AuthorBanner userId={author} onClear={() => update({ author: undefined })} />
      ) : (
        <>
          <SearchBox value={typed} onChange={setTyped} placeholder={t("browse.search")} />
          <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
            {!query && (
              <Tabs<Sort>
                value={sort}
                onChange={(next) => update({ sort: next === "all" ? undefined : next })}
                items={SORTS.map((value) => ({ value, label: t(`browse.sort.${value}`) }))}
              />
            )}
            {catalog.status === "ready" && (
              <p className="text-sm font-bold text-ink-faint">
                {t("browse.count", { count: catalog.emojis.length })}
              </p>
            )}
          </div>
          {(catalog.categories.length > 0 || category !== undefined) && (
            // biome-ignore lint/a11y/useSemanticElements: a row of filter buttons, not a form group.
            <div
              role="group"
              aria-label={t("browse.categories")}
              className="scrollbar-none -mx-4 mt-3 flex gap-1.5 overflow-x-auto px-4 pb-1"
            >
              <FilterPill
                active={category === undefined}
                onClick={() => update({ category: undefined })}
              >
                {t("browse.allCategories")}
              </FilterPill>
              {catalog.categories.map((entry) => (
                <FilterPill
                  key={entry.name}
                  active={category === entry.name}
                  onClick={() => update({ category: entry.name })}
                >
                  {entry.name}
                  <span className="text-xs opacity-70">{entry.emojiCount}</span>
                </FilterPill>
              ))}
              {uncategorized && (
                <FilterPill active={category === ""} onClick={() => update({ category: "" })}>
                  {t("browse.uncategorized")}
                </FilterPill>
              )}
            </div>
          )}
        </>
      )}

      <div className="mt-4">
        {author ? (
          <AuthorResults userId={author} />
        ) : query ? (
          <SearchResults query={query} category={category} />
        ) : sort === "popular" ? (
          <PopularResults keep={inCategory} />
        ) : sort === "new" ? (
          <NewestResults keep={inCategory} />
        ) : catalog.status === "ready" ? (
          <CatalogResults
            emojis={catalog.emojis.filter(inCategory)}
            grouped={category === undefined}
            categories={catalog.categories.map((entry) => entry.name)}
          />
        ) : catalog.status === "error" ? (
          <EmptyState emoji="😵‍💫" title={t("browse.error")} body={t("browse.errorBody")} />
        ) : (
          <TileSkeletons count={24} />
        )}
      </div>

      {catalog.emojis.some((entry) => entry.sensitive) && (
        <div className="mt-6 rounded-2xl bg-surface">
          <SwitchRow
            label={t("browse.showSensitive")}
            hint={t("browse.showSensitiveHint")}
            checked={showSensitive}
            onChange={setShowSensitive}
          />
        </div>
      )}
    </Page>
  );
}

function HeaderLink({
  to,
  label,
  children,
}: {
  to: "/emoji/my-deck" | "/emoji/submissions";
  label: string;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      aria-label={label}
      title={label}
      className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
    >
      {children}
    </Link>
  );
}

function Tiles({ emojis }: { emojis: CatalogEmoji[] }) {
  return (
    <TileGrid>
      {emojis.map((entry) => (
        <EmojiTile key={entry.id} emoji={entry} />
      ))}
    </TileGrid>
  );
}

/** The whole catalogue from this browser's copy: by category when no filter is on. */
function CatalogResults({
  emojis,
  grouped,
  categories,
}: {
  emojis: CatalogEmoji[];
  grouped: boolean;
  categories: string[];
}) {
  const { t } = useTranslation("emoji");
  if (emojis.length === 0) {
    return (
      <EmptyState
        emoji="🫥"
        title={t("browse.empty")}
        body={t("browse.emptyBody")}
        action={
          <Link
            to="/emoji/new-emoji"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            {t("submit.action")}
          </Link>
        }
      />
    );
  }
  return <BoundedResults emojis={emojis} grouped={grouped} categories={categories} />;
}

/** Tiles drawn at first; a catalogue can hold thousands, and the rest follows as the page is scrolled. */
const TILE_CHUNK = 200;

function BoundedResults({
  emojis,
  grouped,
  categories,
}: {
  emojis: CatalogEmoji[];
  grouped: boolean;
  categories: string[];
}) {
  const { t } = useTranslation("emoji");
  const all = grouped
    ? [...categories, ""]
        .map((name) => ({ name, emojis: emojis.filter((entry) => entry.category === name) }))
        .filter((section) => section.emojis.length > 0)
    : [];
  // The filter in force names the list: another one starts from the top again.
  const { shown, sentinel } = useGrowingCount(
    emojis.length,
    TILE_CHUNK,
    `${grouped}/${emojis[0]?.category ?? ""}`,
  );
  if (all.length <= 1) {
    return (
      <>
        <Tiles emojis={emojis.slice(0, shown)} />
        {sentinel}
      </>
    );
  }
  // Sections fill up in order until the budget is spent; a section that got nothing is not started yet.
  let budget = shown;
  const sections = all.flatMap((section) => {
    const part = section.emojis.slice(0, budget);
    budget -= part.length;
    return part.length > 0 ? [{ ...section, part }] : [];
  });
  return (
    <div className="flex flex-col gap-5">
      {sections.map((section) => (
        <section key={section.name} aria-label={section.name || t("browse.uncategorized")}>
          <h2 className="mb-2 flex items-baseline gap-2 px-1 font-black">
            {section.name || t("browse.uncategorized")}
            <span className="text-xs font-bold text-ink-faint">{section.emojis.length}</span>
          </h2>
          <Tiles emojis={section.part} />
        </section>
      ))}
      {sentinel}
    </div>
  );
}

function SearchResults({ query, category }: { query: string; category: string | undefined }) {
  const { t } = useTranslation("emoji");
  // The server ranks equal matches by popularity, which this browser's copy does not know.
  const results = useQuery({
    queryKey: emojiKeys.search(query, category),
    queryFn: async () =>
      (await emojiClient().searchEmoji({ query, category, limit: 100 })).emojis.map(toCatalogEmoji),
    placeholderData: (previous) => previous,
  });
  if (results.isPending) return <TileSkeletons />;
  if (results.isError)
    return <EmptyState emoji="😵‍💫" title={t("browse.error")} body={t("browse.errorBody")} />;
  if (results.data.length === 0)
    return (
      <EmptyState
        emoji="🔍"
        title={t("browse.noResults", { query })}
        body={t("browse.noResultsBody")}
      />
    );
  return <Tiles emojis={results.data} />;
}

function PopularResults({ keep }: { keep(entry: CatalogEmoji): boolean }) {
  const { t } = useTranslation("emoji");
  const popular = useQuery({
    queryKey: emojiKeys.popular,
    queryFn: async () =>
      (await emojiClient().listPopularEmoji({ limit: 100 })).emojis.map(toCatalogEmoji),
  });
  if (popular.isPending) return <TileSkeletons />;
  if (popular.isError)
    return <EmptyState emoji="😵‍💫" title={t("browse.error")} body={t("browse.errorBody")} />;
  const emojis = popular.data.filter(keep);
  if (emojis.length === 0)
    return <EmptyState emoji="🌱" title={t("browse.noPopular")} body={t("browse.noPopularBody")} />;
  return <Tiles emojis={emojis} />;
}

function NewestResults({ keep }: { keep(entry: CatalogEmoji): boolean }) {
  const { t } = useTranslation("emoji");
  const newest = useInfiniteQuery({
    queryKey: emojiKeys.newest,
    queryFn: ({ pageParam }) =>
      emojiClient().listNewestEmoji({ page: { limit: 60, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  if (newest.isPending) return <TileSkeletons />;
  if (newest.isError)
    return <EmptyState emoji="😵‍💫" title={t("browse.error")} body={t("browse.errorBody")} />;
  const emojis = newest.data.pages
    .flatMap((page) => page.emojis)
    .map(toCatalogEmoji)
    .filter(keep);
  return (
    <>
      {emojis.length === 0 && !newest.hasNextPage ? (
        <EmptyState emoji="🫥" title={t("browse.empty")} body={t("browse.emptyBody")} />
      ) : (
        <Tiles emojis={emojis} />
      )}
      <LoadMore query={newest} />
    </>
  );
}

function AuthorBanner({ userId, onClear }: { userId: string; onClear(): void }) {
  const { t } = useTranslation("emoji");
  const profile = useQuery({
    queryKey: profileKeys.byId(userId),
    queryFn: () => profileClient().getProfile({ by: { case: "userId", value: userId } }),
    retry: false,
  });
  const user = profile.data?.profile?.user;
  return (
    <div className="sticker flex items-center gap-3 rounded-card bg-surface p-3">
      <Avatar user={user} size="md" />
      <div className="min-w-0 flex-1">
        <p className="truncate font-black">
          {t("browse.byAuthor", { name: user?.displayName ?? "…" })}
        </p>
        {user && (
          <Link
            to="/u/$handle"
            params={{ handle: user.handle }}
            className="truncate text-sm font-bold text-brand"
          >
            @{user.handle}
          </Link>
        )}
      </div>
      <button
        type="button"
        onClick={onClear}
        aria-label={t("browse.clearAuthor")}
        title={t("browse.clearAuthor")}
        className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2"
      >
        <X className="size-5" />
      </button>
    </div>
  );
}

function AuthorResults({ userId }: { userId: string }) {
  const { t } = useTranslation("emoji");
  const made = useInfiniteQuery({
    queryKey: emojiKeys.author(userId),
    queryFn: ({ pageParam }) =>
      emojiClient().listAuthorEmoji({ userId, page: { limit: 60, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  if (made.isPending) return <TileSkeletons />;
  if (made.isError)
    return <EmptyState emoji="😵‍💫" title={t("browse.error")} body={t("browse.errorBody")} />;
  const emojis = made.data.pages.flatMap((page) => page.emojis).map(toCatalogEmoji);
  if (emojis.length === 0) return <EmptyState emoji="🎨" title={t("browse.authorEmpty")} />;
  return (
    <>
      <Tiles emojis={emojis} />
      <LoadMore query={made} />
    </>
  );
}
