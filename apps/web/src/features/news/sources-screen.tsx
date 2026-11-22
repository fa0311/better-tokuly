import { Link } from "@tanstack/react-router";
import { ChevronRight, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useNewsSources } from "./api";
import { SourceIcon } from "./parts";

/** `/news/sources`: where the articles come from. A source opens the list filtered to it. */
export function SourcesScreen() {
  const { t } = useTranslation("news");
  const sources = useNewsSources();
  const list = sources.data?.sources ?? [];
  return (
    <Page title={t("sources.title")} back="/news">
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("sources.lead")}</p>
      {sources.isPending ? (
        <div className="flex flex-col gap-2" aria-busy>
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
        </div>
      ) : sources.isError ? (
        <EmptyState emoji="🌀" title={errorMessage(sources.error)} />
      ) : list.length === 0 ? (
        <EmptyState emoji="📡" title={t("sources.empty")} body={t("sources.emptyBody")} />
      ) : (
        <ul className="flex flex-col gap-2">
          {list.map((source) => (
            <li
              key={source.id}
              className="flex items-center gap-1 rounded-2xl border-2 border-line bg-surface pr-2 transition-colors hover:border-ink-faint"
            >
              <Link
                to="/news"
                search={{ source: source.id }}
                className="flex min-w-0 flex-1 items-center gap-3 p-3"
              >
                <span className="grid size-11 shrink-0 place-items-center rounded-[0.85rem] bg-surface-2">
                  <SourceIcon source={source} className="size-6" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">{source.name}</span>
                  <span className="block truncate text-xs text-ink-soft">
                    {source.language && (
                      <span className="mr-1.5 rounded bg-surface-2 px-1 font-bold uppercase">
                        {source.language}
                      </span>
                    )}
                    {source.siteUrl.replace(/^https?:\/\/(www\.)?/, "").replace(/\/$/, "")}
                  </span>
                </span>
                <ChevronRight className="size-5 shrink-0 text-ink-faint" />
              </Link>
              {/^https?:\/\//.test(source.siteUrl) && (
                <a
                  href={source.siteUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label={t("sources.openSite", { name: source.name })}
                  className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
                >
                  <ExternalLink className="size-4" />
                </a>
              )}
            </li>
          ))}
        </ul>
      )}
    </Page>
  );
}
