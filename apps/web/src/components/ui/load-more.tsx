import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "./spinner";

/** The part of a TanStack `useInfiniteQuery` result that paging needs. */
export interface PagedQuery {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage(): unknown;
}

/**
 * Put this after the last row of an infinite list: it loads the next page
 * when it scrolls into view and says so when the list is finished.
 */
export function LoadMore({
  query,
  showEnd,
}: {
  query: PagedQuery /** Show the "you reached the end" line (for long feeds). */;
  showEnd?: boolean;
}) {
  const { t } = useTranslation("common");
  const sentinel = useRef<HTMLDivElement>(null);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;

  useEffect(() => {
    const element = sentinel.current;
    if (!element || !hasNextPage || isFetchingNextPage) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) fetchNextPage();
      },
      { rootMargin: "600px 0px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  if (!hasNextPage)
    return showEnd ? (
      <p className="py-6 text-center text-sm font-bold text-ink-faint">{t("state.endOfList")}</p>
    ) : null;
  return (
    <div ref={sentinel} className="grid h-16 place-items-center" aria-busy={isFetchingNextPage}>
      {isFetchingNextPage && <Spinner className="size-6 text-ink-faint" />}
    </div>
  );
}
