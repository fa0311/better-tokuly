import { WifiOff } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useOnline } from "./online";

/**
 * Says so while the device has no connection, and is not there otherwise. A
 * small pill at the top of the screen that taps pass through, so it never
 * blocks what is under it.
 */
export function OfflineIndicator() {
  const { t } = useTranslation("pwa");
  const online = useOnline();
  return (
    // The (empty) status region stays mounted so screen readers announce the pill when it appears.
    <div role="status" className="pointer-events-none absolute inset-x-0 top-0 z-40 flex justify-center px-4 pt-[max(env(safe-area-inset-top),0.5rem)]">
      {!online && (
        <p className="sticker-sm flex animate-pop-in items-center gap-1.5 rounded-full bg-sun px-3 py-1 text-sm font-black text-[#17122b]">
          <WifiOff className="size-4" />
          {t("offline.label")}
        </p>
      )}
    </div>
  );
}
