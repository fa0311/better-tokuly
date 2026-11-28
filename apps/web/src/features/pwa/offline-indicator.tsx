import { useRouterState } from "@tanstack/react-router";
import { WifiOff } from "lucide-react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useOnline } from "./online";

/**
 * Pages whose top edge is a picture, not a header: the video player sits
 * there. On those the pill goes above the tab bar instead.
 */
const TOP_IS_TAKEN = ["/watch/"];

/**
 * Says so while the device has no connection, and is not there otherwise. A
 * small pill that taps pass through, so it never blocks what is under it:
 * inside the header strip at the top of the screen, where no composer or
 * player is.
 */
export function OfflineIndicator() {
  const { t } = useTranslation("pwa");
  const online = useOnline();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const low = TOP_IS_TAKEN.some((prefix) => pathname.startsWith(prefix));
  return (
    // The (empty) status region stays mounted so screen readers announce the pill when it appears.
    <div
      role="status"
      className={cn(
        "pointer-events-none absolute inset-x-0 z-40 flex justify-center px-4",
        low
          ? "bottom-0 pb-[calc(max(env(safe-area-inset-bottom),0.75rem)+5rem)] md:pb-4"
          : "top-0 pt-[max(env(safe-area-inset-top),0.5rem)]",
      )}
    >
      {!online && (
        <p className="sticker-sm flex animate-pop-in items-center gap-1.5 rounded-full bg-sun px-3 py-1 text-sm font-black text-[#17122b]">
          <WifiOff className="size-4" />
          {t("offline.label")}
        </p>
      )}
    </div>
  );
}
