import { createFileRoute } from "@tanstack/react-router";
import { NewsScreen } from "#/features/news/news-screen";
import { parseNewsParams } from "#/features/news/params";

// `/news?order=latest&tag=…&source=…&lang=…`: the order and the filters live in the address.
export const Route = createFileRoute("/_app/news/")({
  validateSearch: (search: Record<string, unknown>) => parseNewsParams(search),
  component: NewsPage,
});

function NewsPage() {
  return <NewsScreen params={Route.useSearch()} />;
}
