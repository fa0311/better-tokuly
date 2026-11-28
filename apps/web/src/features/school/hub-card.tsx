import { Link } from "@tanstack/react-router";
import { classWhen, clock, currentClass } from "@tokuly/core/school";
import { ChevronRight } from "lucide-react";
import { useAccountNow } from "#/lib/account-time";
import { useTranslation } from "#/lib/use-translation";
import { useClassNow } from "./api";

/**
 * The school tile of the "じぶん" hub: leads to the school area and says
 * what class is on, or which one comes next.
 */
export default function SchoolHubCard() {
  const { t, i18n } = useTranslation("school");
  const { data } = useClassNow();
  const { ms: now, date: today } = useAccountNow(30_000);
  const current = currentClass(data, now);
  const next = data?.next;
  const line = current
    ? t("now.tileCurrent", { subject: current.subject?.name ?? "", end: clock(current.endMinute) })
    : next
      ? t("now.tileNext", {
          subject: next.subject?.name ?? "",
          when: classWhen(i18n, next, today),
        })
      : t("now.tileEmpty");
  return (
    <Link
      to="/school"
      className="pressable sticker flex items-center gap-4 rounded-card bg-surface p-4"
    >
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-lime text-2xl">
        🏫
      </span>
      <span className="min-w-0 flex-1">
        <span className="block font-black">{t("title")}</span>
        <span className="block truncate text-sm font-bold text-ink-soft">{line}</span>
      </span>
      <ChevronRight className="size-6 shrink-0 text-ink-faint" />
    </Link>
  );
}
