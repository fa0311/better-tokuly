import { createFileRoute } from "@tanstack/react-router";
import { ConnectionCallbackScreen } from "#/features/settings/connections/connection-callback-screen";

const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);

export const Route = createFileRoute("/_app/settings/connections/callback/$provider")({
  // What an OAuth provider appends when it sends the person back.
  validateSearch: (search): { code?: string; state?: string; error?: string } => ({
    code: text(search.code),
    state: text(search.state),
    error: text(search.error),
  }),
  component: ConnectionCallbackPage,
});

function ConnectionCallbackPage() {
  const { provider } = Route.useParams();
  const { code, state, error } = Route.useSearch();
  return <ConnectionCallbackScreen provider={provider} code={code} state={state} error={error} />;
}
