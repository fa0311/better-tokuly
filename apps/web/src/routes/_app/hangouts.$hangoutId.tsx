import { createFileRoute, redirect } from "@tanstack/react-router";

// `/hangouts/<id>` is the address the server uses for a hangout (notifications, reports, profile
// cards); its page lives with the rooms.
export const Route = createFileRoute("/_app/hangouts/$hangoutId")({
  beforeLoad: ({ params }) => {
    throw redirect({
      to: "/spaces/hangouts/$hangoutId",
      params: { hangoutId: params.hangoutId },
      replace: true,
    });
  },
});
