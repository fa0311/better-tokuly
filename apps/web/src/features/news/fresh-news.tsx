import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { feed } from "@tokuly/api";
import { ChevronRight, Newspaper } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { newsClient, newsKeys } from "./api";
import { PublishedAt, SourceIcon } from "./parts";

/**
 * The newest articles as a short list with a link to the news page: a block
 * for other screens (the explore page). Renders nothing while there is no
 * news.
 */
export function FreshNews({ limit = 4, className }: { limit?: number; className?: string }) {
  const { t } = useTranslation("news");
  const latest = useQuery({
    queryKey: newsKeys.fresh(limit),
    queryFn: () => newsClient().listNews({ page: { limit }, order: feed.NewsOrder.LATEST }),
    staleTime: 60_000,
    retry: false,
  });
  const articles = latest.data?.articles ?? [];
  if (articles.length === 0) return null;
  return (
    <section aria-label={t("fresh.title")} className={className}>
      <header className="mb-2 flex items-center gap-2 px-1">
        <h2 className="flex min-w-0 flex-1 items-center gap-2 text-lg font-black">
          <Newspaper className="size-5 shrink-0 text-ink-soft" />
          <span className="truncate">{t("fresh.title")}</span>
        </h2>
        <Link
          to="/news"
          className="inline-flex shrink-0 items-center text-sm font-bold text-brand hover:underline"
        >
          {t("fresh.more")}
          <ChevronRight className="size-4" />
        </Link>
      </header>
      <ul className="flex flex-col gap-2">
        {articles.map((article) => (
          <li key={article.id}>
            <Link
              to="/news/$articleId"
              params={{ articleId: article.id }}
              className="flex min-w-0 items-center gap-3 rounded-2xl border-2 border-line bg-surface p-3 transition-colors hover:border-ink-faint"
            >
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span
                  className={cn(
                    "line-clamp-2 leading-snug font-black break-words",
                    article.clicked && "text-ink-soft",
                  )}
                >
                  {article.title}
                </span>
                <span className="flex min-w-0 items-center gap-1.5 text-xs text-ink-soft">
                  <SourceIcon source={article.source} />
                  <span className="truncate font-bold">{article.source?.name}</span>
                  <span aria-hidden>·</span>
                  <PublishedAt at={article.publishedAt} className="shrink-0" />
                </span>
              </span>
              {article.imageUrl && (
                <img
                  src={article.imageUrl}
                  alt=""
                  className="size-16 shrink-0 rounded-xl bg-surface-3 object-cover"
                  loading="lazy"
                  decoding="async"
                  draggable={false}
                />
              )}
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}
