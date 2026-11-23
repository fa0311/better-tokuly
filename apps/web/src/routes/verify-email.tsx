import { createFileRoute } from "@tanstack/react-router";
import { MailLinkPage } from "#/components/mail-link-page";

export const Route = createFileRoute("/verify-email")({
  validateSearch: (search): { token?: string } => ({
    token: typeof search.token === "string" ? search.token : undefined,
  }),
  component: VerifyEmail,
});

/** Where an address verification (or address change) link leads. */
function VerifyEmail() {
  const { token } = Route.useSearch();
  return <MailLinkPage token={token} kind="verify" />;
}
