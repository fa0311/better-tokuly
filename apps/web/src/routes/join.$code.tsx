import { createFileRoute } from "@tanstack/react-router";
import { InviteScreen } from "#/features/community/invite-screen";

/**
 * `/join/<invite code>`: what an invite link or QR code opens. It shows the
 * community the invite leads to — also to someone without an account, who
 * is sent through sign-up and comes back here — so it lives outside the
 * signed-in layout and renders in the browser only.
 */
export const Route = createFileRoute("/join/$code")({ ssr: false, component: InvitePage });

function InvitePage() {
  const { code } = Route.useParams();
  return <InviteScreen key={code} code={code} />;
}
