import { createFileRoute } from "@tanstack/react-router";
import { NotesScreen, parseNotesSearch } from "#/features/notes/notes-screen";
import { StoryTray } from "#/features/stories";

/** The notes home. `?tab=home|global|mentions|feed` selects a timeline, so going back returns to it. */
export const Route = createFileRoute("/_app/notes/")({
  validateSearch: parseNotesSearch,
  component: NotesPage,
});

function NotesPage() {
  const { tab } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <NotesScreen
      tab={tab}
      onTabChange={(next) => void navigate({ search: { tab: next }, replace: true })}
      top={<StoryTray />}
    />
  );
}
