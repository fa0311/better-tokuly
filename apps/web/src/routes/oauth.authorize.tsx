import { createFileRoute } from "@tanstack/react-router";
import { ConsentScreen } from "#/features/dev/oauth/consent-screen";
import { NO_FRAMING } from "#/features/dev/oauth/headers";

/**
 * `/oauth/authorize`: the OAuth authorization endpoint, the consent page an
 * application sends people to. It lives outside the signed-in layout (a
 * visitor signs in first and comes back), renders in the browser only (the
 * session lives there), and may never be shown inside a frame.
 */
export const Route = createFileRoute("/oauth/authorize")({
  ssr: false,
  headers: () => NO_FRAMING,
  component: ConsentScreen,
});
