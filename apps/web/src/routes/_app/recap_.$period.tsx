import { createFileRoute, redirect } from "@tanstack/react-router";
import { parseRecapSearch } from "#/features/notes/feed/recap-screen";

/**
 * `/recap/2026-10` (or `/recap/2026`): the address the "your recap is ready"
 * notification points at. It is the same recap as `/recap?p=2026-10`, so it
 * simply goes there; anything that is not a month or a year opens this month.
 */
export const Route = createFileRoute("/_app/recap_/$period")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/recap", search: parseRecapSearch({ p: params.period }), replace: true });
  },
});
