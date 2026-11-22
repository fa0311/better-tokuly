import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { search } from "@tokuly/api";
import { ArrowUpDown, Check, ChevronRight, Hash, RotateCw, X } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { profileClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { refreshRecentSearches, searchClient, searchKeys, submitSearch, useTrending, wasSubmitted } from "./api";
import { formatCount } from "./format";
import { HitCard, HitSkeleton } from "./hit-card";
import { isKind, KINDS, type Kind, kindFromSlug, kindSlug } from "./kinds";
import { fromTagQuery, isTagQuery, type SearchParams, SORTS, type Sort, sortValue, toTagQuery } from "./params";

const ALL = search.SearchKind.UNSPECIFIED;
/** Hits asked for in the "all" tab (the API's maximum): enough to show the best of every kind. */
const ALL_LIMIT = 50;
const PAGE_LIMIT = 20;
/** Results of one kind shown in its section of the "all" tab. */
const SECTION_SIZE = 4;

/** The parameters of a result page: `q` is there. */
export type ResultParams = SearchParams & { q: string };

/**
 * The results of a query: an "all" tab with the best of every kind in
 * sections, and a tab per kind that has matches, each an endless list.
 */
export function Results({ params, onChange }: { params: ResultParams; /** Moves to another result page (a new address). */ onChange(next: ResultParams): void }) {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const kind = kindFromSlug(params.kind) ?? ALL;
  const list = useInfiniteQuery({
    queryKey: searchKeys.results(params.q, kind, params.sort, params.owner),
    queryFn: async ({ pageParam, signal }) => {
      // Only searches the person made themselves are remembered (see `markSubmitted`).
      const skipHistory = !wasSubmitted(params.q);
      const response = await searchClient().search(
        { query: params.q, kind, sort: sortValue(params.sort), ownerId: params.owner ?? "", page: { cursor: pageParam, limit: kind === ALL ? ALL_LIMIT : PAGE_LIMIT }, skipHistory },
        { signal },
      );
      if (!pageParam && !skipHistory) refreshRecentSearches(queryClient);
      return response;
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });

  // The counts belong to the query, not to the tab: they stay while another tab loads.
  const scope = `${params.q}\n${params.owner ?? ""}`;
  const [known, setKnown] = useState<{ scope: string; counts: search.KindCount[] }>();
  const fresh = list.data?.pages[0]?.counts;
  if (fresh && (known?.scope !== scope || known.counts !== fresh)) setKnown({ scope, counts: fresh });
  const counts = new Map<search.SearchKind, number>((known?.scope === scope ? known.counts : []).map((entry) => [entry.kind, Number(entry.count)]));

  const hits = list.data?.pages.flatMap((page) => page.hits) ?? [];
  const total = Number(list.data?.pages[0]?.total ?? 0);

  return (
    <section aria-label={t("results.label", { query: params.q })}>
      <KindTabs params={params} kind={kind} counts={counts} />
      <Filters params={params} onChange={onChange} total={list.isSuccess && hits.length > 0 ? total : undefined} ownerHint={hits.find((hit) => hit.owner?.id === params.owner)?.owner} />
      {list.isPending ? (
        <div className="grid gap-2 lg:grid-cols-2" aria-busy>
          {Array.from({ length: 6 }, (_, index) => (
            <HitSkeleton key={index} />
          ))}
        </div>
      ) : list.isError ? (
        <Problem error={list.error} onRetry={() => void list.refetch()} />
      ) : hits.length === 0 ? (
        <NoResults params={params} kind={kind} counts={counts} />
      ) : kind === ALL ? (
        <Sections params={params} hits={hits} counts={counts} />
      ) : (
        <>
          <HitGrid hits={hits} />
          <LoadMore query={list} showEnd={hits.length > PAGE_LIMIT} />
        </>
      )}
    </section>
  );
}

function HitGrid({ hits }: { hits: search.SearchHit[] }) {
  return (
    <ul className="grid gap-2 lg:grid-cols-2">
      {hits.map((hit) => (
        <li key={`${hit.kind}:${hit.id}`} className="min-w-0">
          <HitCard hit={hit} className="h-full" />
        </li>
      ))}
    </ul>
  );
}

// ───────────────────────────── Tabs ─────────────────────────────

/** "All" plus a tab for every kind the query matches. Tabs are links: the open tab lives in the address. */
function KindTabs({ params, kind, counts }: { params: ResultParams; kind: search.SearchKind; counts: Map<search.SearchKind, number> }) {
  const { t } = useTranslation("search");
  const kinds = KINDS.filter((candidate) => counts.has(candidate) || candidate === kind);
  const tab = (value: search.SearchKind, label: string, count?: number) => {
    const active = value === kind;
    return (
      <Link
        key={value}
        to="/search"
        search={{ ...params, kind: value === ALL ? undefined : kindSlug(value as Kind) }}
        // Only the tab that is this very address counts as the current page.
        activeOptions={{ exact: true }}
        role="tab"
        aria-selected={active}
        className={cn("flex h-9 shrink-0 items-center gap-1.5 rounded-full px-4 text-sm font-bold whitespace-nowrap transition-colors", active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink")}
      >
        {label}
        {count !== undefined && <span className={cn("text-xs tabular-nums", active ? "text-ink-soft" : "text-ink-faint")}>{formatCount(count)}</span>}
      </Link>
    );
  };
  return (
    <div role="tablist" aria-label={t("results.tabs")} className="scrollbar-none -mx-4 mb-3 flex gap-1 overflow-x-auto px-4 pb-1 md:mx-0 md:px-0">
      <div className="flex gap-1 rounded-full bg-surface-2 p-1">
        {tab(ALL, t("kinds.all"))}
        {kinds.map((candidate) => tab(candidate, t(`kinds.${kindSlug(candidate)}`), counts.get(candidate)))}
      </div>
    </div>
  );
}

// ───────────────────────────── Sort and filters ─────────────────────────────

const chip = "inline-flex h-8 shrink-0 items-center gap-1 rounded-full border-2 px-3 text-xs font-black whitespace-nowrap transition-colors";
const chipOff = "border-line bg-surface text-ink-soft hover:border-ink-faint hover:text-ink";
const chipOn = "border-ink bg-ink text-surface";

/** Sort order, "tags only" and the owner filter, with the number of matches. */
function Filters({ params, onChange, total, ownerHint }: { params: ResultParams; onChange(next: ResultParams): void; total: number | undefined; ownerHint: search.SearchHit["owner"] }) {
  const { t } = useTranslation("search");
  const tagsOnly = isTagQuery(params.q);
  const sortLabel = (sort: Sort | undefined) => t(`sort.${sort ?? "relevance"}`);
  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      <Menu
        align="start"
        trigger={
          <button type="button" aria-label={t("sort.label", { order: sortLabel(params.sort) })} className={cn(chip, params.sort ? chipOn : chipOff)}>
            <ArrowUpDown className="size-3.5" />
            {sortLabel(params.sort)}
          </button>
        }
        actions={([undefined, ...SORTS] as (Sort | undefined)[]).map((sort) => ({
          label: sortLabel(sort),
          icon: sort === params.sort ? <Check /> : <span />,
          onSelect: () => onChange({ ...params, sort }),
        }))}
      />
      <button type="button" aria-pressed={tagsOnly} onClick={() => onChange({ ...params, q: tagsOnly ? fromTagQuery(params.q) : toTagQuery(params.q) })} className={cn(chip, tagsOnly ? chipOn : chipOff)}>
        <Hash className="size-3.5" />
        {t("filters.tagsOnly")}
      </button>
      {params.owner && <OwnerChip params={params} hint={ownerHint} />}
      {total !== undefined && <p className="ml-auto text-xs font-bold text-ink-faint">{t("results.total", { count: total, formatted: total.toLocaleString() })}</p>}
    </div>
  );
}

/** "Only posts by this person": who it is, and a way to lift the filter. */
function OwnerChip({ params, hint }: { params: ResultParams; hint: search.SearchHit["owner"] }) {
  const { t } = useTranslation("search");
  const owner = useQuery({
    queryKey: ["identity", "users", params.owner],
    queryFn: async () => (await profileClient().batchGetUsers({ userIds: [params.owner as string] })).users[0] ?? null,
    enabled: !hint,
    staleTime: 5 * 60_000,
  });
  const user = hint ?? owner.data ?? undefined;
  const name = user ? user.displayName || `@${user.handle}` : t("filters.someone");
  return (
    <span className={cn(chip, chipOn, "pr-1")}>
      {user && <Avatar user={user} size="xs" className="-ml-1.5" />}
      <span className="max-w-40 truncate">{t("filters.owner", { name })}</span>
      <Link to="/search" search={{ ...params, owner: undefined }} aria-label={t("filters.clearOwner")} className="grid size-6 place-items-center rounded-full hover:bg-surface/20">
        <X className="size-3.5" />
      </Link>
    </span>
  );
}

// ───────────────────────────── The "all" tab ─────────────────────────────

/** The best of every kind, in the order the engine ranked each kind's best match. */
function Sections({ params, hits, counts }: { params: ResultParams; hits: search.SearchHit[]; counts: Map<search.SearchKind, number> }) {
  const { t } = useTranslation("search");
  const order: Kind[] = [];
  const byKind = new Map<Kind, search.SearchHit[]>();
  for (const hit of hits) {
    if (!isKind(hit.kind)) continue;
    if (!byKind.has(hit.kind)) {
      byKind.set(hit.kind, []);
      order.push(hit.kind);
    }
    byKind.get(hit.kind)?.push(hit);
  }
  // Kinds that match but whose results rank below everything on this page still have their tab.
  const elsewhere = KINDS.filter((kind) => counts.has(kind) && !byKind.has(kind));
  return (
    <div className="flex flex-col gap-6">
      {order.map((kind) => {
        const all = byKind.get(kind) ?? [];
        const count = Math.max(counts.get(kind) ?? 0, all.length);
        const shown = all.slice(0, SECTION_SIZE);
        const name = t(`kinds.${kindSlug(kind)}`);
        return (
          <section key={kind} aria-label={name}>
            <header className="mb-2 flex items-baseline gap-2 px-1">
              <h2 className="text-lg font-black">{name}</h2>
              <span className="text-sm font-bold text-ink-faint tabular-nums">{formatCount(count)}</span>
              {count > shown.length && (
                <Link to="/search" search={{ ...params, kind: kindSlug(kind) }} className="ml-auto inline-flex items-center text-sm font-bold text-brand hover:underline">
                  {t("results.seeAll")}
                  <ChevronRight className="size-4" />
                </Link>
              )}
            </header>
            <ul className="grid gap-2 lg:grid-cols-2">
              {shown.map((hit) => (
                <li key={hit.id} className="min-w-0">
                  <HitCard hit={hit} className="h-full" />
                </li>
              ))}
            </ul>
          </section>
        );
      })}
      {elsewhere.length > 0 && (
        <section aria-label={t("results.elsewhere")}>
          <h2 className="mb-2 px-1 text-sm font-black text-ink-soft">{t("results.elsewhere")}</h2>
          <KindLinks params={params} kinds={elsewhere} counts={counts} />
        </section>
      )}
    </div>
  );
}

function KindLinks({ params, kinds, counts }: { params: ResultParams; kinds: Kind[]; counts: Map<search.SearchKind, number> }) {
  const { t } = useTranslation("search");
  return (
    <ul className="flex flex-wrap gap-2">
      {kinds.map((kind) => (
        <li key={kind}>
          <Link to="/search" search={{ ...params, kind: kindSlug(kind) }} className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-3.5 text-sm font-bold">
            {t(`kinds.${kindSlug(kind)}`)}
            <span className="text-xs text-ink-faint tabular-nums">{formatCount(counts.get(kind) ?? 0)}</span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

// ───────────────────────────── Nothing found, or something broke ─────────────────────────────

/** An empty result is never a dead end: other tabs that do match, ways to widen the search, what others look for. */
function NoResults({ params, kind, counts }: { params: ResultParams; kind: search.SearchKind; counts: Map<search.SearchKind, number> }) {
  const { t } = useTranslation("search");
  const queryClient = useQueryClient();
  const trending = useTrending();
  const otherKinds = KINDS.filter((candidate) => candidate !== kind && counts.has(candidate));
  const tagsOnly = isTagQuery(params.q);
  const ideas: ReactNode[] = [];
  if (otherKinds.length > 0) {
    ideas.push(
      <Idea key="tabs" title={t("empty.otherTabs")}>
        <KindLinks params={params} kinds={otherKinds} counts={counts} />
      </Idea>,
    );
  }
  if (params.owner) {
    ideas.push(
      <Idea key="owner" title={t("empty.ownerFiltered")}>
        <Link to="/search" search={{ ...params, owner: undefined }} className="pressable sticker-sm inline-flex h-9 items-center rounded-full bg-surface px-3.5 text-sm font-bold">
          {t("empty.searchEveryone")}
        </Link>
      </Idea>,
    );
  }
  if (tagsOnly) {
    ideas.push(
      <Idea key="tags" title={t("empty.tagsOnly")}>
        <Link to="/search" search={{ ...params, q: fromTagQuery(params.q) }} className="pressable sticker-sm inline-flex h-9 items-center rounded-full bg-surface px-3.5 text-sm font-bold">
          {t("empty.searchEverything", { query: fromTagQuery(params.q) })}
        </Link>
      </Idea>,
    );
  }
  const trends = (trending.data?.queries ?? []).filter((item) => item.query.toLowerCase() !== params.q.toLowerCase()).slice(0, 8);
  return (
    <div className="mx-auto flex max-w-xl flex-col items-center gap-2 px-2 py-10 text-center" data-testid="search-empty">
      <div className="animate-float text-6xl">🔍</div>
      <p className="mt-2 text-lg font-black break-words">{t("empty.title", { query: params.q })}</p>
      <ul className="flex flex-col gap-1 text-sm leading-relaxed text-ink-soft">
        <li>{t("empty.tipSpelling")}</li>
        <li>{t("empty.tipFewer")}</li>
      </ul>
      <div className="mt-4 flex w-full flex-col gap-5 text-left">
        {ideas}
        {trends.length > 0 && (
          <Idea title={t("empty.trending")}>
            <ul className="flex flex-wrap gap-2">
              {trends.map((item) => (
                <li key={item.query}>
                  <Link to="/search" search={{ q: item.query }} onClick={() => submitSearch(queryClient, item.query)} className="pressable sticker-sm inline-flex h-9 max-w-full items-center rounded-full bg-surface px-3.5 text-sm font-bold">
                    <span className="truncate">{item.query}</span>
                  </Link>
                </li>
              ))}
            </ul>
          </Idea>
        )}
        <Idea title={t("empty.elsewhere")}>
          <ul className="flex flex-wrap gap-2">
            {(
              [
                { to: "/search", label: t("empty.explore") },
                { to: "/friends", label: t("empty.friends") },
                { to: "/stamps", label: t("empty.stamps") },
                { to: "/news", label: t("empty.news") },
              ] as const
            ).map((place) => (
              <li key={place.to}>
                <Link to={place.to} className="pressable sticker-sm inline-flex h-9 items-center rounded-full bg-surface px-3.5 text-sm font-bold">
                  {place.label}
                </Link>
              </li>
            ))}
          </ul>
        </Idea>
      </div>
    </div>
  );
}

function Idea({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h2 className="mb-2 text-sm font-black">{title}</h2>
      {children}
    </section>
  );
}

function Problem({ error, onRetry }: { error: unknown; onRetry(): void }) {
  const { t } = useTranslation("common");
  return (
    <div role="alert" className="flex flex-col items-center gap-3 px-6 py-12 text-center">
      <div className="text-5xl">🌀</div>
      <p className="max-w-sm font-bold">{errorMessage(error)}</p>
      <Button size="sm" variant="secondary" icon={<RotateCw className="size-4" />} onClick={onRetry}>
        {t("actions.retry")}
      </Button>
    </div>
  );
}
