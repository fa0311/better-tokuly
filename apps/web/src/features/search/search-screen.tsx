import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { kindFromSlug } from "@tokuly/core/search";
import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import { submitSearch } from "./api";
import { Explore } from "./explore";
import { useHasKeyboard } from "./hooks";
import { RecentPanel, TrendingPanel } from "./panels";
import type { SearchParams } from "./params";
import { Results } from "./results";
import { SearchBox } from "./search-box";

/**
 * `/search`: one field over everything. Without a query it is the explore
 * page; with one, the results. What is shown is decided by the address alone.
 */
export function SearchScreen({ params }: { params: SearchParams }) {
  const { t } = useTranslation("search");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const keyboard = useHasKeyboard();
  const query = params.q;

  // A new query keeps the open tab and the owner filter, and starts from the default order.
  const runSearch = (next: string) => {
    submitSearch(queryClient, next);
    void navigate({ to: "/search", search: { q: next, kind: params.kind, owner: params.owner } });
  };

  const box = (
    <SearchBox
      className="mb-5"
      query={query ?? ""}
      kind={kindFromSlug(params.kind)}
      onSearch={runSearch}
      autoFocus={!query && keyboard}
    />
  );
  return (
    <Page title={t("title")} wide>
      {query ? (
        <div className="xl:grid xl:grid-cols-[minmax(0,1fr)_19rem] xl:gap-8">
          <div className="min-w-0">
            {box}
            <Results
              params={{ ...params, q: query }}
              onChange={(next) => void navigate({ to: "/search", search: next })}
            />
          </div>
          <aside className="hidden flex-col gap-7 pt-1 xl:flex">
            <TrendingPanel limit={8} />
            <RecentPanel />
          </aside>
        </div>
      ) : (
        <>
          <div className="mx-auto max-w-3xl">{box}</div>
          <Explore />
        </>
      )}
    </Page>
  );
}
