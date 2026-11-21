import { useTranslation } from "react-i18next";
import { FilterSelect } from "./filters";

/** Lengths a suspension can be given, in seconds (the API takes 1 hour to 365 days). */
export const SUSPENSION_PRESETS = [3600, 21_600, 86_400, 3 * 86_400, 7 * 86_400, 14 * 86_400, 30 * 86_400, 90 * 86_400, 365 * 86_400] as const;

/** "1時間" / "7日" for a suspension length. */
export function useDurationLabel(): (seconds: number) => string {
  const { t } = useTranslation("admin");
  return (seconds) => (seconds < 86_400 ? t("common.hours", { count: Math.round(seconds / 3600) }) : t("common.days", { count: Math.round(seconds / 86_400) }));
}

/** How long a suspension lasts. */
export function DurationSelect({ value, onChange }: { value: number; onChange(seconds: number): void }) {
  const { t } = useTranslation("admin");
  const label = useDurationLabel();
  return (
    <FilterSelect
      label={t("common.suspensionLength")}
      value={String(value)}
      onChange={(next) => onChange(Number(next))}
      options={SUSPENSION_PRESETS.map((seconds) => ({ value: String(seconds), label: label(seconds) }))}
    />
  );
}
