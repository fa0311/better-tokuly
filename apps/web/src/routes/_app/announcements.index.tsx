import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { AnnouncementsScreen } from "#/features/announcements/announcements-screen";
import {
  ANNOUNCEMENT_CATEGORIES,
  type AnnouncementCategory,
} from "#/features/announcements/categories";

export const Route = createFileRoute("/_app/announcements/")({
  validateSearch: (search): { category?: AnnouncementCategory } => ({
    category: ANNOUNCEMENT_CATEGORIES.find((category) => category === Number(search.category)),
  }),
  component: AnnouncementsPage,
});

function AnnouncementsPage() {
  const { category } = Route.useSearch();
  const navigate = useNavigate();
  return (
    <AnnouncementsScreen
      category={category}
      onCategoryChange={(next) =>
        navigate({ to: "/announcements", search: { category: next }, replace: true })
      }
    />
  );
}
