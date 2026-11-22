import { createFileRoute, redirect } from "@tanstack/react-router";

// `/notes/tags/<tag>` is the address the server uses for a hashtag (search results); the page lives at `/tag/<tag>`.
export const Route = createFileRoute("/_app/notes/tags/$tag")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/tag/$tag", params: { tag: params.tag }, replace: true });
  },
});
