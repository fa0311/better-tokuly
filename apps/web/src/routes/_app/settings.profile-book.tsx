import { createFileRoute } from "@tanstack/react-router";
import { ProfileBookSection } from "#/features/settings/profile-book/profile-book-section";

export const Route = createFileRoute("/_app/settings/profile-book")({
  component: ProfileBookSection,
});
