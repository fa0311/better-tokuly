import type { shorts } from "@tokuly/api";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState } from "#/components/ui/page";
import { useTranslation } from "#/lib/use-translation";
import { ShortList } from "./short-tile";

/** The public remixes and duets made of a short. */
export function RemixesDialog({ short, onClose }: { short: shorts.Short; onClose(): void }) {
  const { t } = useTranslation("shorts");
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("remixes.title")}>
      <ShortList
        context={{ kind: "remixes", key: short.shortId }}
        empty={<EmptyState emoji="🎛️" title={t("remixes.empty")} />}
      />
    </Dialog>
  );
}
