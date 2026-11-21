import { createFileRoute } from "@tanstack/react-router";
import { InboxScreen, type InboxTab } from "#/features/notifications/inbox-screen";

/** The inbox. `?tab=unread` / `?tab=c:<category>` selects a filter, so going back returns to it. */
export const Route = createFileRoute("/_app/notifications/")({
  validateSearch: (search: Record<string, unknown>): { tab?: InboxTab } => {
    const tab = typeof search.tab === "string" ? search.tab : "";
    return tab === "unread" || tab.startsWith("c:") ? { tab: tab as InboxTab } : {};
  },
  component: InboxPage,
});

function InboxPage() {
  const { tab = "all" } = Route.useSearch();
  const navigate = Route.useNavigate();
  return <InboxScreen tab={tab} onTabChange={(next) => navigate({ search: next === "all" ? {} : { tab: next }, replace: true })} />;
}
