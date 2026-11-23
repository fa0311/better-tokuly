import { createFileRoute } from "@tanstack/react-router";
import { OverlayScreen } from "#/features/live/overlay/overlay-screen";

/**
 * `/overlay/<token>`: one OBS widget, added to OBS as a browser source. It
 * needs no account — the token in the address is the widget's secret — so it
 * lives outside the signed-in layout, renders in the browser only and shows
 * nothing but the widget on a transparent page.
 *
 * Because the address is the secret, it must never travel on as a referrer:
 * the response says `Referrer-Policy: no-referrer` and the document repeats
 * it for browsers that only read the tag. Everything the page loads comes
 * from this origin.
 */
export const Route = createFileRoute("/overlay/$token")({
  ssr: false,
  headers: () => ({
    "Referrer-Policy": "no-referrer",
    "Cache-Control": "no-store",
    "X-Robots-Tag": "noindex, nofollow",
  }),
  head: () => ({
    meta: [
      { title: "Tokuly overlay" },
      { name: "referrer", content: "no-referrer" },
      { name: "robots", content: "noindex, nofollow" },
    ],
  }),
  component: OverlayPage,
});

function OverlayPage() {
  const { token } = Route.useParams();
  return <OverlayScreen key={token} token={token} />;
}
