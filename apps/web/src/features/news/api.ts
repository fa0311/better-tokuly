import {
  type InfiniteData,
  type QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { feed } from "@tokuly/api";
import { toast } from "sonner";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import type { NewsParams } from "./params";

export const newsClient = () => api().service(feed.NewsService);
export const newsAdminClient = () => api().service(feed.NewsAdminService);

/** Query keys of the news half of the feed domain. */
export const newsKeys = {
  all: ["feed", "news"] as const,
  lists: ["feed", "news", "list"] as const,
  list: (params: NewsParams) =>
    [
      "feed",
      "news",
      "list",
      params.order ?? "recommended",
      params.tag ?? "",
      params.source ?? "",
      params.lang ?? "",
    ] as const,
  fresh: (limit: number) => ["feed", "news", "list", "fresh", limit] as const,
  related: (tag: string) => ["feed", "news", "list", "related", tag] as const,
  article: (articleId: string) => ["feed", "news", "article", articleId] as const,
  tags: ["feed", "news", "tags"] as const,
  sources: ["feed", "news", "sources"] as const,
  admin: { sources: ["feed", "news", "admin", "sources"] as const },
};

/** The sources articles come from (for filters and the sources page). */
export function useNewsSources() {
  return useQuery({
    queryKey: newsKeys.sources,
    queryFn: () => newsClient().listNewsSources({}),
    staleTime: 5 * 60_000,
  });
}

/** The reader's interests and the tags most common among recent articles. */
export function useNewsTags(limit = 20) {
  return useQuery({
    queryKey: [...newsKeys.tags, limit],
    queryFn: () => newsClient().listNewsTags({ limit }),
  });
}

// ───────────────────────────── Cache patching ─────────────────────────────

type ListPage = Awaited<ReturnType<ReturnType<typeof newsClient>["listNews"]>>;

const isInfinite = (data: unknown): data is InfiniteData<ListPage> =>
  typeof data === "object" && data !== null && Array.isArray((data as { pages?: unknown }).pages);

/** Applies a change to one article wherever it is cached: every list and its own page. */
export function patchArticle(
  queryClient: QueryClient,
  articleId: string,
  update: (article: feed.NewsArticle) => feed.NewsArticle,
): void {
  const patchList = <T extends { articles: feed.NewsArticle[] }>(page: T): T =>
    page.articles.some((article) => article.id === articleId)
      ? {
          ...page,
          articles: page.articles.map((article) =>
            article.id === articleId ? update(article) : article,
          ),
        }
      : page;
  for (const [key, data] of queryClient.getQueriesData({ queryKey: newsKeys.lists })) {
    if (isInfinite(data))
      queryClient.setQueryData(key, { ...data, pages: data.pages.map(patchList) });
    else if (data && Array.isArray((data as ListPage).articles))
      queryClient.setQueryData(key, patchList(data as ListPage));
  }
  queryClient.setQueryData(
    newsKeys.article(articleId),
    (data: { article?: feed.NewsArticle } | undefined) =>
      data?.article ? { ...data, article: update(data.article) } : data,
  );
}

/** Takes an article out of every cached list (after staff removed it). */
export function dropArticle(queryClient: QueryClient, articleId: string): void {
  const dropFrom = <T extends { articles: feed.NewsArticle[] }>(page: T): T =>
    page.articles.some((article) => article.id === articleId)
      ? { ...page, articles: page.articles.filter((article) => article.id !== articleId) }
      : page;
  for (const [key, data] of queryClient.getQueriesData({ queryKey: newsKeys.lists })) {
    if (isInfinite(data))
      queryClient.setQueryData(key, { ...data, pages: data.pages.map(dropFrom) });
    else if (data && Array.isArray((data as ListPage).articles))
      queryClient.setQueryData(key, dropFrom(data as ListPage));
  }
  queryClient.removeQueries({ queryKey: newsKeys.article(articleId) });
}

// ───────────────────────────── Reading and feedback ─────────────────────────────

/**
 * Tells the API that the reader opened an article. The first time, it raises
 * their interest in the article's tags; the lists mark it as read at once.
 * Failing to record is not the reader's problem: nothing is shown.
 */
export function recordClick(
  queryClient: QueryClient,
  article: Pick<feed.NewsArticle, "id" | "clicked">,
): void {
  const first = !article.clicked;
  patchArticle(queryClient, article.id, (current) =>
    current.clicked ? current : { ...current, clicked: true, clickCount: current.clickCount + 1n },
  );
  newsClient()
    .recordNewsClick({ articleId: article.id })
    .then(() => {
      if (first) void queryClient.invalidateQueries({ queryKey: newsKeys.tags });
    })
    .catch(() => {});
}

/** "More like this" / "less like this"; choosing the current answer again takes it back. Shows at once, rolls back on failure. */
export function useFeedback() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { article: feed.NewsArticle; feedback: feed.NewsFeedback }) =>
      newsClient().sendNewsFeedback({ articleId: input.article.id, feedback: input.feedback }),
    onMutate: ({ article, feedback }) => {
      patchArticle(queryClient, article.id, (current) => ({ ...current, feedback }));
      return { before: article.feedback };
    },
    onError: (error, { article }, context) => {
      patchArticle(queryClient, article.id, (current) => ({
        ...current,
        feedback: context?.before ?? feed.NewsFeedback.UNSPECIFIED,
      }));
      toast.error(errorMessage(error));
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: newsKeys.tags }),
  });
}
