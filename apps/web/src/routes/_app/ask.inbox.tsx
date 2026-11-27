import { createFileRoute } from "@tanstack/react-router";
import { InboxScreen } from "#/features/ask/inbox-screen";
import { INBOX_TABS, type InboxTab } from "#/features/ask/inbox-search";

/** The owner's inbox (`ask.question` notifications lead here). `?tab=answered|archived|filtered` selects a folder. */
export const Route = createFileRoute("/_app/ask/inbox")({
  validateSearch: (search: Record<string, unknown>): { tab?: InboxTab } => {
    const tab = INBOX_TABS.find((value) => value === search.tab);
    return tab && tab !== "inbox" ? { tab } : {};
  },
  component: InboxPage,
});

function InboxPage() {
  const { tab = "inbox" } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <InboxScreen
      tab={tab}
      onTabChange={(next) =>
        navigate({ search: next === "inbox" ? {} : { tab: next }, replace: true })
      }
    />
  );
}
