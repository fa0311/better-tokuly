import { createFileRoute } from "@tanstack/react-router";
import { PublicBoxScreen } from "#/features/ask/public-box-screen";

/**
 * `/q/<handle>`: the link people put in their stories and bios. It shows the
 * question box and its public answers without an account, so it lives outside
 * the signed-in layout and renders in the browser only. `?a=<question id>`
 * brings one shared answer to the top.
 */
export const Route = createFileRoute("/q/$handle")({
  ssr: false,
  validateSearch: (search: Record<string, unknown>): { a?: string } => (typeof search.a === "string" && search.a ? { a: search.a } : {}),
  component: PublicBoxPage,
});

function PublicBoxPage() {
  const { handle } = Route.useParams();
  const { a } = Route.useSearch();
  return <PublicBoxScreen key={handle} handle={handle} answerId={a} />;
}
