import { createFileRoute } from "@tanstack/react-router";
import { MailLinkPage } from "#/components/mail-link-page";

export const Route = createFileRoute("/undo-email-change")({
  validateSearch: (search): { token?: string } => ({
    token: typeof search.token === "string" ? search.token : undefined,
  }),
  component: UndoEmailChange,
});

/** Where the "move it back" link mailed to a previous address leads. */
function UndoEmailChange() {
  const { token } = Route.useSearch();
  return <MailLinkPage token={token} kind="undo" />;
}
