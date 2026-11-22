import { createFileRoute } from "@tanstack/react-router";
import { PartyScreen } from "#/features/video/party/party-screen";

/** `/party/<join code>`: where party invitations lead for people with an account. */
export const Route = createFileRoute("/_app/party/$partyId")({ component: PartyPage });

function PartyPage() {
  const { partyId } = Route.useParams();
  return <PartyScreen key={partyId} code={partyId} />;
}
