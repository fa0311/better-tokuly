import { createFileRoute } from "@tanstack/react-router";
import { PartyGuestScreen } from "#/features/video/party/guest-screen";

/**
 * `/join/party/<join code>`: the link people share to bring others into a
 * watch party. It needs no account — guests join with a name — so it lives
 * outside the signed-in layout and renders in the browser only.
 */
export const Route = createFileRoute("/join/party/$code")({ ssr: false, component: GuestPartyPage });

function GuestPartyPage() {
  const { code } = Route.useParams();
  return <PartyGuestScreen key={code} code={code} />;
}
