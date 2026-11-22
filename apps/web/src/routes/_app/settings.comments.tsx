import { createFileRoute } from "@tanstack/react-router";
import {
  CommentModerationScreen,
  MODERATION_TABS,
  type ModerationTab,
} from "#/features/comments/moderation-screen";

/** The comment manager. `?tab=` keeps the open tab in the address, so going back returns to it. */
export const Route = createFileRoute("/_app/settings/comments")({
  validateSearch: (search: Record<string, unknown>): { tab?: ModerationTab } => {
    const tab = MODERATION_TABS.find((candidate) => candidate === search.tab);
    return tab && tab !== "queue" ? { tab } : {};
  },
  component: CommentsPage,
});

function CommentsPage() {
  const { tab = "queue" } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <CommentModerationScreen
      tab={tab}
      onTabChange={(next) =>
        navigate({ search: next === "queue" ? {} : { tab: next }, replace: true })
      }
    />
  );
}
