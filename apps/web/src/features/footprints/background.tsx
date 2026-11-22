import { useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { realtime } from "#/lib/realtime";
import { footprintKeys, regionName } from "./api";
import { CelebrationLayer, celebrate } from "./celebrate";

/** New cells are announced at most this often: a walk uncovers one every minute or so. */
const CELL_TOAST_EVERY_MS = 5 * 60_000;
/** The footprint queries are re-read at most this often while cells keep coming in. */
const REFRESH_EVERY_MS = 4_000;

/**
 * Runs while someone is signed in: when the server says they uncovered new
 * cells, the footprint numbers everywhere refresh, a region entered for the
 * first time is celebrated, and new cells get a (rare) toast that leads to
 * the footprint map.
 */
export default function FootprintsBackground() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useTranslation("footprints");

  useEffect(() => {
    let pendingCells = 0;
    let lastCellToast = 0;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;

    const off = realtime().onFrame((frame) => {
      if (frame.body.case !== "geo" || frame.body.value.event.case !== "footprintProgress") return;
      const progress = frame.body.value.event.value;
      refreshTimer ??= setTimeout(() => {
        refreshTimer = undefined;
        void queryClient.invalidateQueries({ queryKey: footprintKeys.all });
      }, REFRESH_EVERY_MS);

      const onFootprints = router.state.location.pathname.startsWith("/footprints");
      const open = { label: t("live.open"), onClick: () => void router.navigate({ to: "/footprints" }) };
      if (progress.newRegion) {
        celebrate(["🎉", "🚩", "✨", "🗺️"]);
        toast.success(t("live.newRegion", { region: regionName(progress.newRegion) }), onFootprints ? {} : { action: open });
      }
      pendingCells += progress.newCells.length;
      // On the footprint page the map itself shows the new cells.
      if (pendingCells > 0 && !onFootprints && !progress.newRegion && Date.now() - lastCellToast >= CELL_TOAST_EVERY_MS) {
        lastCellToast = Date.now();
        toast(t("live.newCells", { count: pendingCells }), { action: open });
        pendingCells = 0;
      }
      if (onFootprints) pendingCells = 0;
    });
    return () => {
      off();
      clearTimeout(refreshTimer);
    };
  }, [queryClient, router, t]);

  return <CelebrationLayer />;
}
