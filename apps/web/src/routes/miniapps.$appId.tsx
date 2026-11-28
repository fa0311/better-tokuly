import { createFileRoute } from "@tanstack/react-router";
import { NO_FRAMING } from "#/features/dev/oauth/headers";
import { MiniAppScreen, type MiniAppSearch } from "#/features/miniapps/screen";

const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);

/**
 * `/miniapps/<id>`: runs a mini app. Outside the signed-in layout (a full
 * screen of its own), rendered in the browser only, and never framed by
 * another site. Its Content-Security-Policy lets it frame the origin `o`
 * names (`src/start.ts`), so it is always reached by a page load.
 */
export const Route = createFileRoute("/miniapps/$appId")({
  ssr: false,
  headers: () => NO_FRAMING,
  validateSearch: (search: Record<string, unknown>): MiniAppSearch => ({
    p: text(search.p),
    c: text(search.c),
    o: text(search.o),
  }),
  component: MiniAppPage,
});

function MiniAppPage() {
  const { appId } = Route.useParams();
  return <MiniAppScreen appId={appId} search={Route.useSearch()} />;
}
