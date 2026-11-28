import { BellRing, X } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { useTranslation } from "#/lib/use-translation";
import { promptSnoozed, snoozePrompt } from "./push";
import { usePush } from "./use-push";

/**
 * The inbox's nudge to turn push on. It only appears when one tap would do
 * it, the browser's permission prompt opens from that tap alone, and once
 * waved away it stays away for a month.
 */
export function PushPrompt() {
  const { t } = useTranslation("notifications");
  const push = usePush();
  const [snoozed, setSnoozed] = useState(promptSnoozed);
  if (snoozed || push.status !== "off") return null;

  const dismiss = () => {
    snoozePrompt();
    setSnoozed(true);
  };

  return (
    <section
      aria-label={t("push.prompt.title")}
      className="sticker relative mb-4 flex animate-pop-in items-start gap-3 rounded-card bg-lime p-4 pr-12 text-[#17122b]"
    >
      <span className="sticker-sm grid size-11 shrink-0 place-items-center rounded-2xl bg-white">
        <BellRing className="size-6 animate-wiggle" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-black leading-snug">{t("push.prompt.title")}</p>
        <p className="mt-0.5 text-sm font-normal leading-snug">{t("push.prompt.body")}</p>
        <Button
          size="sm"
          className="mt-3"
          loading={push.enable.isPending}
          onClick={() => push.enable.mutate()}
        >
          {t("push.prompt.enable")}
        </Button>
      </div>
      <button
        type="button"
        aria-label={t("push.prompt.dismiss")}
        onClick={dismiss}
        className="absolute top-2 right-2 grid size-9 place-items-center rounded-full hover:bg-black/10"
      >
        <X className="size-5" />
      </button>
    </section>
  );
}
