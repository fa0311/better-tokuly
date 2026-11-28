import { createFileRoute } from "@tanstack/react-router";
import { AppearanceSection } from "#/features/settings/appearance/appearance-section";

export const Route = createFileRoute("/_app/settings/appearance/")({
  component: AppearanceSection,
});
