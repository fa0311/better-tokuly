import { notify } from "@tokuly/api";
import { Eye } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Card } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { usePreferenceActions } from "../preferences";

/** The three choices, in the order they are offered: most shown first. */
const CHOICES = [
  { value: notify.PushPreview.FULL, key: "full" },
  { value: notify.PushPreview.SENDER_ONLY, key: "senderOnly" },
  { value: notify.PushPreview.HIDDEN, key: "hidden" },
] as const;

/**
 * 「通知に内容を表示」: what a push shows on the lock screen. Each choice
 * comes with a sample of how a message would look, because the difference is
 * easier to see than to read. What a choice leaves out is never sent to the
 * device at all; the inbox keeps everything.
 */
export function PreviewCard({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const actions = usePreferenceActions();
  const current = preferences.pushPreview || notify.PushPreview.FULL;

  return (
    <Card>
      <div className="flex items-start gap-3">
        <span className="sticker-sm grid size-12 shrink-0 place-items-center rounded-2xl bg-sky text-[#17122b]">
          <Eye className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-black leading-snug">{t("preview.title")}</h2>
          <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("preview.lead")}</p>
        </div>
      </div>
      <fieldset className="mt-4 flex flex-col gap-2">
        <legend className="sr-only">{t("preview.title")}</legend>
        {CHOICES.map((choice) => {
          const checked = current === choice.value;
          return (
            <label
              key={choice.key}
              className={cn(
                "pressable flex cursor-pointer items-start gap-3 rounded-2xl border-2 p-3",
                checked ? "border-ink bg-lime/30" : "border-line bg-surface-2",
              )}
            >
              <input
                type="radio"
                name="push-preview"
                className="mt-0.5 size-5 shrink-0 cursor-pointer appearance-none rounded-full border-2 border-ink bg-surface checked:border-[6px]"
                checked={checked}
                onChange={() => actions.setPushPreview.mutate(choice.value)}
              />
              <span className="min-w-0 flex-1">
                <span className="block font-black">{t(`preview.choices.${choice.key}.label`)}</span>
                <span className="mt-0.5 block text-xs text-ink-soft">
                  {t(`preview.choices.${choice.key}.hint`)}
                </span>
                <span
                  aria-hidden
                  className="mt-2 block rounded-xl bg-surface px-3 py-2 text-xs shadow-sm"
                >
                  <span className="block font-bold">
                    {t(`preview.choices.${choice.key}.sampleTitle`)}
                  </span>
                  <span className="block text-ink-soft">
                    {t(`preview.choices.${choice.key}.sampleBody`)}
                  </span>
                </span>
              </span>
            </label>
          );
        })}
      </fieldset>
      <p className="mt-3 text-xs text-ink-soft">{t("preview.inbox")}</p>
    </Card>
  );
}
