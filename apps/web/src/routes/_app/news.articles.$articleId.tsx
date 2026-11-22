import { createFileRoute, redirect } from "@tanstack/react-router";

// `/news/articles/<id>` is the address the server uses for an article (in search results, the
// feed, shared links); its page in this app is `/news/<id>`.
export const Route = createFileRoute("/_app/news/articles/$articleId")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/news/$articleId", params: { articleId: params.articleId }, replace: true });
  },
});
