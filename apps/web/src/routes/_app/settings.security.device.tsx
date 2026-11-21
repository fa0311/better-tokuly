import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { DeviceLoginScreen } from "#/features/settings/security/device-login-screen";

export const Route = createFileRoute("/_app/settings/security/device")({
  // `?code=ABCD-2345`, as in the link (or QR code) a device shows.
  validateSearch: (search): { code?: string } => ({ code: typeof search.code === "string" && search.code.trim() ? search.code.trim() : undefined }),
  component: DeviceLoginPage,
});

function DeviceLoginPage() {
  const { code } = Route.useSearch();
  const navigate = useNavigate();
  return <DeviceLoginScreen code={code} onCodeChange={(next) => navigate({ to: "/settings/security/device", search: { code: next }, replace: true })} />;
}
