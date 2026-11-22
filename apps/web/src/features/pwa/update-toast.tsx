import { Sparkles, X } from "lucide-react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { applyUpdate, pageIsBehind, useWorker } from "./service-worker";

const TOAST_ID = "pwa:update";

/**
 * While a newer version waits, a toast offers to switch to it. It stays until
 * it is answered: "update" reloads into the new version, "later" leaves the
 * running one alone (the settings page keeps offering the update).
 */
export function useUpdateToast(): void {
  // Waiting for the go-ahead, already switched to from another tab, or outrun by the server: either way this page is behind.
  const updateReady = useWorker(pageIsBehind);
  useEffect(() => {
    if (!updateReady) {
      toast.dismiss(TOAST_ID);
      return;
    }
    toast.custom(() => <UpdateToast />, { id: TOAST_ID, duration: Number.POSITIVE_INFINITY });
  }, [updateReady]);
}

function UpdateToast() {
  const { t } = useTranslation("pwa");
  return (
    <div className="flex w-full items-center gap-3 p-3">
      <span className="sticker-sm grid size-10 shrink-0 place-items-center rounded-2xl bg-lime text-[#17122b]">
        <Sparkles className="size-5" />
      </span>
      <p className="min-w-0 flex-1 font-black leading-snug">{t("update.toast")}</p>
      <Button size="sm" onClick={applyUpdate}>
        {t("update.reload")}
      </Button>
      <button type="button" aria-label={t("update.later")} onClick={() => toast.dismiss(TOAST_ID)} className="grid size-8 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2">
        <X className="size-4" />
      </button>
    </div>
  );
}
