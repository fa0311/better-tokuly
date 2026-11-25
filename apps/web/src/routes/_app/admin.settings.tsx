import { createFileRoute } from "@tanstack/react-router";
import { RequireAccess } from "#/features/admin/console-layout";
import {
  parseSettingsSearch,
  ServerSettingsScreen,
} from "#/features/admin/settings/settings-screen";

export const Route = createFileRoute("/_app/admin/settings")({
  validateSearch: parseSettingsSearch,
  component: SettingsPage,
});

function SettingsPage() {
  return (
    <RequireAccess level="admin">
      <ServerSettingsScreen search={Route.useSearch()} />
    </RequireAccess>
  );
}
