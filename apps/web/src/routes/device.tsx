import { createFileRoute, redirect } from "@tanstack/react-router";

/**
 * The short address shown on a TV or console that wants to sign in
 * (`/device?code=ABCD-EFGH`). Approving happens in the security settings.
 */
export const Route = createFileRoute("/device")({
  validateSearch: (search): { code?: string } => ({ code: typeof search.code === "string" ? search.code : undefined }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/settings/security/device", search: { code: search.code }, replace: true });
  },
});
