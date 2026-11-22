import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Heart, Megaphone, RotateCw, Rss, SlidersHorizontal, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { newsClient, newsKeys, useNewsSources, useNewsTags } from "./api";
import { ArticleCard, ArticleSkeleton } from "./article-card";
import { listRequest, type NewsParams } from "./params";
import { SourceIcon } from "./parts";

const PAGE_SIZE = 20;

const chip = "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full border-2 px-3.5 text-sm font-bold whitespace-nowrap transition-colors";
const chipOff = "border-line bg-surface text-ink-soft hover:border-ink-faint hover:text-ink";
const chipOn = "border-ink bg-ink text-surface";

/**
 * `/news`: articles ranked for the reader (or newest first), with tags as
 * filter chips. The order and the filters live in the address.
 */
export function NewsScreen({ params }: { params: NewsParams }) {
  const { t } = useTranslation("news");
  const list = useInfiniteQuery({
    queryKey: newsKeys.list(params),
    queryFn: ({ pageParam }) => newsClient().listNews(listRequest(params, pageParam, PAGE_SIZE)),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const articles = list.data?.pages.flatMap((page) => page.articles) ?? [];
  // A tag or a source makes the list newest-first; a language only narrows it, in either order.
  const filtered = Boolean(params.tag || params.source);
  const narrowed = filtered || Boolean(params.lang);
  // Filtered lists are always newest first; the API says which order a page is in.
  const recommended = list.data?.pages[0]?.recommended ?? (!filtered && params.order !== "latest");

  return (
    <Page
      title={t("title")}
      wide
      actions={
        <>
          <Link to="/announcements" className="pressable sticker-sm inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full bg-surface px-3 text-sm font-bold" aria-label={t("header.announcements")}>
            <Megaphone className="size-4" />
            <span className="hidden sm:inline">{t("header.announcements")}</span>
          </Link>
          <Link to="/news/interests" aria-label={t("header.interests")} title={t("header.interests")} className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2">
            <SlidersHorizontal className="size-5" />
          </Link>
          <Link to="/news/sources" aria-label={t("header.sources")} title={t("header.sources")} className="grid size-9 shrink-0 place-items-center rounded-full hover:bg-surface-2">
            <Rss className="size-5" />
          </Link>
        </>
      }
    >
      <Filters params={params} recommended={recommended} filtered={filtered} />
      {list.isPending ? (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3" aria-busy>
          {Array.from({ length: 6 }, (_, index) => (
            <ArticleSkeleton key={index} />
          ))}
        </div>
      ) : list.isError ? (
        <EmptyState
          emoji="🌀"
          title={errorMessage(list.error)}
          action={
            <Button size="sm" variant="secondary" icon={<RotateCw className="size-4" />} onClick={() => void list.refetch()}>
              {t("retry")}
            </Button>
          }
        />
      ) : articles.length === 0 ? (
        narrowed ? (
          <EmptyState
            emoji="🗞️"
            title={t("empty.filteredTitle")}
            body={t("empty.filteredBody")}
            action={
              <Link to="/news" search={{ order: params.order }} className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
                {t("empty.clear")}
              </Link>
            }
          />
        ) : (
          <EmptyState
            emoji="📰"
            title={t("empty.title")}
            body={t("empty.body")}
            action={
              <Link to="/announcements" className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand">
                <Megaphone className="size-4" />
                {t("header.announcements")}
              </Link>
            }
          />
        )
      ) : (
        <>
          <ul className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            {articles.map((article) => (
              <li key={article.id} className="min-w-0">
                <ArticleCard article={article} />
              </li>
            ))}
          </ul>
          <LoadMore query={list} showEnd={articles.length > 8} />
        </>
      )}
    </Page>
  );
}

/** Order, tag chips and the active source / language filters. */
function Filters({ params, recommended, filtered }: { params: NewsParams; recommended: boolean; filtered: boolean }) {
  const { t } = useTranslation("news");
  const tags = useNewsTags();
  const sources = useNewsSources();
  const source = params.source ? sources.data?.sources.find((candidate) => candidate.id === params.source) : undefined;

  // The reader's interests first, then what is common right now; the chosen tag is always among them.
  const interests = (tags.data?.interests ?? []).map((interest) => interest.tag);
  const names = [...new Set([...(params.tag ? [params.tag] : []), ...interests.slice(0, 6), ...(tags.data?.popular ?? []).map((entry) => entry.tag)])].slice(0, 24);
  const languages = [...new Set((sources.data?.sources ?? []).map((candidate) => candidate.language).filter(Boolean))].sort();

  return (
    <div className="mb-4 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div role="tablist" aria-label={t("order.label")} className="inline-flex gap-1 rounded-full bg-surface-2 p-1">
          {(["recommended", "latest"] as const).map((order) => {
            const active = (order === "recommended") === recommended;
            return (
              <Link
                key={order}
                to="/news"
                // Choosing an order starts over from the whole list: filtered lists only come newest first.
                search={order === "latest" ? { ...params, order: "latest" } : { lang: params.lang }}
                activeOptions={{ exact: true }}
                role="tab"
                aria-selected={active}
                className={cn("flex h-9 items-center rounded-full px-4 text-sm font-bold transition-colors", active ? "sticker-sm bg-surface text-ink" : "text-ink-soft hover:text-ink")}
              >
                {t(`order.${order}`)}
              </Link>
            );
          })}
        </div>
        {source && (
          <span className={cn(chip, chipOn, "pr-1")}>
            <SourceIcon source={source} />
            <span className="max-w-40 truncate">{source.name}</span>
            <Link to="/news" search={{ ...params, source: undefined }} aria-label={t("filters.clearSource")} className="grid size-6 place-items-center rounded-full hover:bg-surface/20">
              <X className="size-3.5" />
            </Link>
          </span>
        )}
        {languages.length > 1 && (
          <fieldset className="flex min-w-0 items-center gap-1" aria-label={t("filters.language")}>
            {languages.map((language) => {
              const active = params.lang === language;
              return (
                <Link key={language} to="/news" search={{ ...params, lang: active ? undefined : language }} activeOptions={{ exact: true }} aria-current={active ? "page" : undefined} className={cn(chip, "h-8 px-2.5 text-xs uppercase", active ? chipOn : chipOff)}>
                  {language}
                </Link>
              );
            })}
          </fieldset>
        )}
        {filtered && !recommended && <p className="text-xs font-bold text-ink-faint">{t("order.filteredHint")}</p>}
      </div>
      {names.length > 0 && (
        <nav aria-label={t("filters.tags")} className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1 md:mx-0 md:flex-wrap md:px-0">
          {/* A chip is the current page when its filter is the one applied; tapping the applied one lifts it. */}
          <Link to="/news" search={{ ...params, tag: undefined }} activeOptions={{ exact: true }} aria-current={params.tag ? undefined : "page"} className={cn(chip, params.tag ? chipOff : chipOn)}>
            {t("filters.allTags")}
          </Link>
          {names.map((tag) => {
            const active = params.tag === tag;
            return (
              <Link key={tag} to="/news" search={{ ...params, tag: active ? undefined : tag }} activeOptions={{ exact: true }} aria-current={active ? "page" : undefined} className={cn(chip, active ? chipOn : chipOff)}>
                {interests.includes(tag) && <Heart className={cn("size-3.5", active ? "fill-current" : "fill-pink text-pink")} aria-label={t("filters.interest")} />}#{tag}
              </Link>
            );
          })}
        </nav>
      )}
    </div>
  );
}

