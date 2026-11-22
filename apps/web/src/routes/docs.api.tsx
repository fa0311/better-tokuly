import { createFileRoute } from "@tanstack/react-router";
import { ReferenceLayout } from "#/features/dev/reference/reference-screen";

/**
 * `/docs/api`: the public API reference. No account needed, so it lives
 * outside the signed-in layout; it is drawn from what the API says about
 * itself, in the browser.
 */
export const Route = createFileRoute("/docs/api")({
  ssr: false,
  head: () => ({ meta: [{ title: "Tokuly API" }] }),
  component: ReferenceLayout,
});
