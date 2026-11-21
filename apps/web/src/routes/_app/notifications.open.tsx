import { createFileRoute } from "@tanstack/react-router";
import { OpenNotification } from "#/features/notifications/open-notification";

/**
 * `/notifications/open?path=<target>&id=<notification>`: where the service
 * worker sends a tap on a system notification when no page of the app is open.
 */
export const Route = createFileRoute("/_app/notifications/open")({
  validateSearch: (search: Record<string, unknown>) => ({
    path: typeof search.path === "string" ? search.path : "",
    // The router reads an all-digit value as a number.
    id: typeof search.id === "string" || typeof search.id === "number" ? String(search.id) : "",
  }),
  component: OpenPage,
});

function OpenPage() {
  const { path, id } = Route.useSearch();
  return <OpenNotification path={path} id={id} />;
}
