import { useQuery } from "@tanstack/react-query";
import { CircleCheck, CircleMinus, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { flagClient } from "../api";
import { decisionKey } from "../labels";
import { Section } from "../ui/console-page";
import { ErrorState } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { UserName } from "../ui/user-name";
import { UserPicker } from "../ui/user-picker";

/** "Evaluate for a user": which flags one account gets, and why. */
export function EvaluatePanel({ userId, onChange }: { userId?: string; onChange(userId: string | undefined): void }) {
  const { t } = useTranslation("admin");
  const evaluation = useQuery({
    queryKey: ["admin", "flags", "evaluate", userId],
    queryFn: () => flagClient().evaluateFlags({ userId }),
    enabled: Boolean(userId),
  });

  return (
    <Section title={t("flags.evaluate.title")}>
      <div className="flex flex-col gap-3 p-4">
        <p className="text-sm text-ink-soft">{t("flags.evaluate.lead")}</p>
        {userId ? (
          <div className="flex items-center gap-2 rounded-control border-2 border-brand bg-brand-soft py-1.5 pr-1.5 pl-3 text-sm">
            <span className="min-w-0 flex-1">
              <UserName userId={userId} />
            </span>
            <button type="button" aria-label={t("flags.evaluate.clear")} onClick={() => onChange(undefined)} className="grid size-7 shrink-0 place-items-center rounded-full hover:bg-surface">
              <X className="size-4" />
            </button>
          </div>
        ) : (
          <UserPicker label={t("flags.evaluate.pick")} onPick={(account) => onChange(account.user?.id)} />
        )}
      </div>
      {userId &&
        (evaluation.isPending ? (
          <div className="flex flex-col gap-2 border-t-2 border-line p-4" aria-busy>
            <Skeleton className="h-10" />
            <Skeleton className="h-10" />
          </div>
        ) : evaluation.isError ? (
          <div className="border-t-2 border-line">
            <ErrorState error={evaluation.error} onRetry={() => evaluation.refetch()} />
          </div>
        ) : evaluation.data.evaluations.length === 0 ? (
          <p className="border-t-2 border-line px-4 py-4 text-sm text-ink-faint">{t("flags.evaluate.noFlags")}</p>
        ) : (
          <ul aria-label={t("flags.evaluate.results")} className="divide-y divide-line border-t-2 border-line">
            {evaluation.data.evaluations.map((entry) => (
              <li key={entry.name} className="flex items-center gap-3 px-4 py-2.5 text-sm">
                <span className="min-w-0 flex-1">
                  <code className="block truncate font-mono font-bold">{entry.name}</code>
                  <span className="block text-xs text-ink-soft">{t(decisionKey(entry.decision))}</span>
                </span>
                <ToneChip tone={entry.on ? "good" : "neutral"} icon={entry.on ? <CircleCheck /> : <CircleMinus />}>
                  {t(entry.on ? "flags.on" : "flags.off")}
                </ToneChip>
              </li>
            ))}
          </ul>
        ))}
    </Section>
  );
}
