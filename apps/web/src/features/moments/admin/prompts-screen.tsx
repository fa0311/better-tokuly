import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { moments, toDate } from "@tokuly/api";
import { Zap } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { useStaffAccess } from "#/features/admin";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";

const adminClient = () => api().service(moments.MomentAdminService);
const REGIONS_KEY = ["moments", "admin", "regions"] as const;

/** The places of each region (`rules::BANDS` in the backend), west to east. */
const PLACES: Record<string, string> = {
  "utc-11..-7": "pacific",
  "utc-6..-2": "americas",
  "utc-1..+3": "europe",
  "utc+3:30..+6:30": "southAsia",
  "utc+7..+11": "eastAsia",
  "utc+12..+14": "oceania",
};

/**
 * `/admin/moments`: the six prompt regions with today's day, and the button
 * that sends a region's prompt of today now (once per region and day).
 */
export function PromptsScreen() {
  const { t } = useTranslation("moments");
  const access = useStaffAccess();
  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("staff.title")}</h1>
        </div>
      </header>
      <div className="px-4 pt-4 pb-32 md:pb-10">
        {access.admin ? (
          <Regions />
        ) : (
          <EmptyState emoji="🔐" title={t("staff.gate.title")} body={t("staff.gate.body")} />
        )}
      </div>
    </div>
  );
}

function Regions() {
  const { t, i18n } = useTranslation("moments");
  const queryClient = useQueryClient();
  const [asking, setAsking] = useState<moments.PromptRegion>();
  const regions = useQuery({
    queryKey: REGIONS_KEY,
    queryFn: () => adminClient().listPromptRegions({}),
  });
  const placeOf = (region: string) =>
    PLACES[region] ? t(`staff.places.${PLACES[region]}`) : region;
  const send = useMutation({
    mutationFn: (region: string) => adminClient().sendPromptNow({ region }),
    onSuccess: (_, region) => toast.success(t("staff.sent", { place: placeOf(region) })),
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => {
      setAsking(undefined);
      return queryClient.invalidateQueries({ queryKey: REGIONS_KEY });
    },
  });
  const time = new Intl.DateTimeFormat(i18n.language, { hour: "2-digit", minute: "2-digit" });

  if (regions.isPending) return <Skeleton className="h-96" />;
  if (regions.isError) return <EmptyState emoji="🌀" title={errorMessage(regions.error)} />;

  return (
    <div className="flex max-w-3xl flex-col gap-4">
      <p className="text-sm text-ink-soft">{t("staff.lead")}</p>
      <section className="overflow-clip rounded-2xl border-2 border-line bg-surface">
        <h2 className="border-b-2 border-line px-4 py-2 text-sm font-black">
          {t("staff.regions")}
        </h2>
        <ul aria-label={t("staff.regions")} className="divide-y-2 divide-line">
          {regions.data.regions.map((row) => {
            const sentAt = toDate(row.sentAt);
            return (
              <li
                key={row.region}
                data-testid="prompt-region"
                aria-label={placeOf(row.region)}
                className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 text-sm"
              >
                <div className="min-w-0 flex-1 basis-56">
                  <p className="font-black">{placeOf(row.region)}</p>
                  <p className="font-mono text-xs text-ink-faint">
                    {row.region} · {row.date}
                  </p>
                </div>
                {sentAt ? (
                  <p className="font-bold">
                    {t("staff.sentAt", { at: time.format(sentAt) })}
                    <span className="ml-2 text-xs text-ink-soft">
                      {t(row.sentBy ? "staff.byStaff" : "staff.byPlanner")}
                    </span>
                  </p>
                ) : (
                  <>
                    <p className="font-bold text-ink-soft">{t("staff.notYet")}</p>
                    <Button
                      size="sm"
                      icon={<Zap className="size-4" />}
                      onClick={() => setAsking(row)}
                    >
                      {t("staff.sendNow")}
                    </Button>
                  </>
                )}
              </li>
            );
          })}
        </ul>
      </section>
      <ConfirmDialog
        open={asking !== undefined}
        onOpenChange={(open) => !open && !send.isPending && setAsking(undefined)}
        title={t("staff.confirmTitle", { place: asking ? placeOf(asking.region) : "" })}
        description={t("staff.confirmBody")}
        confirmLabel={t("staff.confirm")}
        loading={send.isPending}
        onConfirm={() => asking && send.mutate(asking.region)}
      />
    </div>
  );
}
