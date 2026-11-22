import { createFileRoute } from "@tanstack/react-router";
import { QuestionScreen } from "#/features/ask/question-screen";

/** `/ask/q/<id>`: the address the server uses for a question in notifications, search, the feed and share links. `?comment=<id>` brings a comment to the top. */
export const Route = createFileRoute("/_app/ask/q/$questionId")({
  validateSearch: (search: Record<string, unknown>): { comment?: string } =>
    typeof search.comment === "string" && search.comment ? { comment: search.comment } : {},
  component: QuestionPage,
});

function QuestionPage() {
  const { questionId } = Route.useParams();
  const { comment } = Route.useSearch();
  return <QuestionScreen key={questionId} questionId={questionId} focusCommentId={comment} />;
}
