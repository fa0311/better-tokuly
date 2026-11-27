import { useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Outlet, useNavigate } from "@tanstack/react-router";
import { Suspense, useEffect } from "react";
import { AppShell } from "#/components/app-shell";
import { BackgroundFeatures } from "#/components/background-features";
import { LogoMark } from "#/components/ui/logo";
import { BirthdateGate } from "#/features/settings";
import { BadgeSourceRunner } from "#/seams/badges";
import { useSessionStatus } from "#/stores/session";

/**
 * Everything under this layout needs a signed-in user. Data lives behind the
 * access token, which only exists in the browser, so these routes render on
 * the client.
 */
export const Route = createFileRoute("/_app")({ ssr: false, component: AppLayout });

function AppLayout() {
  const status = useSessionStatus();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  // Cached answers belong to the account that asked: none may outlive its session.
  useEffect(() => {
    if (status === "signedOut") queryClient.clear();
  }, [status, queryClient]);

  // Send signed-out visitors to the login page, remembering where they were
  // headed. This runs once per sign-out: reading the address inside the effect
  // (not during render) keeps the redirect from feeding on itself.
  useEffect(() => {
    if (status !== "signedOut") return;
    const destination = window.location.pathname + window.location.search;
    void navigate({ to: "/login", search: { redirect: destination }, replace: true });
  }, [status, navigate]);

  if (status !== "signedIn") return <Splash />;
  // The page shows once the background features listen: nothing they handle is missed.
  return (
    <BirthdateGate>
      <Suspense fallback={<Splash />}>
        <AppShell>
          <Outlet />
          <BackgroundFeatures />
          <BadgeSourceRunner />
        </AppShell>
      </Suspense>
    </BirthdateGate>
  );
}

/** Shown for the moment it takes to restore the session. */
function Splash() {
  return (
    <div className="grid h-dvh place-items-center bg-bg">
      <LogoMark className="size-20 animate-float" />
    </div>
  );
}
