import { createFileRoute } from "@tanstack/react-router";
import { ThemeEditorScreen } from "#/features/settings/appearance/theme-editor-screen";

export const Route = createFileRoute("/_app/settings/appearance/theme")({
  component: ThemeEditorScreen,
});
