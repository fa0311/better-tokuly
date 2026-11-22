import { createFileRoute } from "@tanstack/react-router";
import { ArticleScreen } from "#/features/news/article-screen";

export const Route = createFileRoute("/_app/news/$articleId")({ component: ArticlePage });

function ArticlePage() {
  const { articleId } = Route.useParams();
  return <ArticleScreen key={articleId} articleId={articleId} />;
}
