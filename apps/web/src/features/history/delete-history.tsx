import { useMutation, useQueryClient } from "@tanstack/react-query";
import { create, fromDate, geo } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { deviceTimeZone, placeKeys } from "#/features/places";
import { errorMessage } from "#/lib/errors";
import { historyClient, historyKeys } from "./api";
import { dayRange, formatDay } from "./dates";

type Scope = "range" | "all";

/** Every footprint query (the footprints feature builds on this one and cannot be imported from here). */
const FOOTPRINT_KEYS = ["geo", "footprints"] as const;

/**
 * Deleting history for good: one day (`date` given), or — from the history
 * page — a range of days or everything. Always behind this confirmation;
 * footprints uncovered in the range go only when asked.
 */
export function DeleteHistoryDialog({ open, onOpenChange, date, onDeleted }: { open: boolean; onOpenChange(open: boolean): void; /** Delete just this day. */ date?: string; onDeleted?(): void }) {
  const { t } = useTranslation("history");
  const queryClient = useQueryClient();
  const today = formatDay(new Date());
  const [scope, setScope] = useState<Scope>("range");
  const [from, setFrom] = useState(today);
  const [to, setTo] = useState(today);
  const [footprints, setFootprints] = useState(false);

  const start = dayRange(date ?? from)?.from;
  const end = dayRange(date ?? to)?.to;
  const everything = !date && scope === "all";
  const valid = everything || Boolean(start && end && start < end);

  const remove = useMutation({
    mutationFn: () => historyClient().deleteHistory(everything ? { includeFootprints: footprints } : { from: fromDate(start!), to: fromDate(end!), includeFootprints: footprints }),
    onSuccess: async () => {
      // What was deleted must not be shown again from memory, not even for the
      // moment a re-read takes. Resetting (rather than marking stale) empties
      // every cached answer about the history, the footprints and the places
      // (which count visits) at once, tells the screens showing them, and
      // re-reads what is on screen.
      for (const queryKey of [historyKeys.all, FOOTPRINT_KEYS, placeKeys.all]) void queryClient.resetQueries({ queryKey });
      if (date) {
        // The day on screen is empty now; the re-read adds back what remains (a memo stays).
        queryClient.setQueryData(historyKeys.day(date, deviceTimeZone()), create(geo.GetDayResponseSchema, {}));
        queryClient.setQueryData(historyKeys.route(date), create(geo.GetRouteResponseSchema, {}));
      }
      onOpenChange(false);
      toast.success(t("delete.done"));
      onDeleted?.();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "history" })),
  });

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={onOpenChange}
      title={date ? t("delete.dayTitle") : t("delete.title")}
      description={everything ? t("delete.allBody") : t("delete.body")}
      confirmLabel={everything ? t("delete.confirmAll") : t("delete.confirm")}
      danger
      loading={remove.isPending}
      onConfirm={() => valid && remove.mutate()}
    >
      <div className="flex flex-col gap-4">
        {!date && (
          <>
            <Tabs<Scope>
              items={[
                { value: "range", label: t("delete.range") },
                { value: "all", label: t("delete.all") },
              ]}
              value={scope}
              onChange={setScope}
            />
            {scope === "range" && (
              <div className="grid grid-cols-2 gap-3">
                <TextField type="date" label={t("delete.from")} value={from} max={today} onChange={(event) => setFrom(event.target.value)} />
                <TextField type="date" label={t("delete.to")} value={to} max={today} error={valid ? undefined : t("delete.invalidRange")} onChange={(event) => setTo(event.target.value)} />
              </div>
            )}
          </>
        )}
        <SwitchRow label={t("delete.footprints")} hint={t("delete.footprintsHint")} checked={footprints} onChange={setFootprints} />
        <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">⚠️ {t("delete.warning")}</p>
      </div>
    </ConfirmDialog>
  );
}
