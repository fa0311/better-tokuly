import { geo } from "@tokuly/api";
import { Trophy } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { regionName } from "./api";
import { formatPercent, regionLevel } from "./levels";
import { flagEmoji, Meter } from "./meter";

type Kind = "admin1" | "country";

/**
 * Percent explored per prefecture and per country, each with its level and
 * the way to the next one. `onPick` (the owner's own list) makes a row lead
 * to that region's ranking.
 */
export function RegionList({ stats, onPick, coach }: { stats: geo.RegionStat[]; onPick?(regionId: string): void; /** Say how far the next level is (only for one's own numbers). */ coach?: boolean }) {
  const { t } = useTranslation("footprints");
  const [kind, setKind] = useState<Kind>("admin1");
  const admin1 = stats.filter((stat) => stat.region?.kind === geo.RegionKind.ADMIN1);
  const countries = stats.filter((stat) => stat.region?.kind === geo.RegionKind.COUNTRY);
  const shown = kind === "admin1" && admin1.length > 0 ? admin1 : countries;
  if (stats.length === 0) return null;

  return (
    <section aria-label={t("regions.title")}>
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-black">{t("regions.title")}</h2>
        {admin1.length > 0 && countries.length > 0 && (
          <Tabs<Kind>
            items={[
              { value: "admin1", label: t("regions.admin1") },
              { value: "country", label: t("regions.country") },
            ]}
            value={kind}
            onChange={setKind}
          />
        )}
      </div>
      <ul className="mt-3 flex flex-col gap-3">
        {shown.map((stat) => {
          const region = stat.region!;
          const level = regionLevel(stat.percent);
          const body = (
            <>
              <div className="flex items-center gap-3">
                <span className="text-3xl" aria-hidden>
                  {flagEmoji(region.countryCode)}
                </span>
                <div className="min-w-0 flex-1 text-left">
                  <p className="truncate font-black">{regionName(region)}</p>
                  <p className="text-xs font-bold text-ink-faint">{t("regions.cells", { count: Number(stat.cellCount), km2: stat.exploredKm2.toFixed(stat.exploredKm2 < 10 ? 1 : 0) })}</p>
                </div>
                <div className="text-right">
                  <p className="text-xl font-black tabular-nums">{formatPercent(stat.percent)}</p>
                  <Chip className="bg-brand-soft text-brand-strong">Lv.{level.level}</Chip>
                </div>
              </div>
              <Meter className="mt-3" value={level.progress} label={t("regions.progressLabel", { region: regionName(region) })} />
              <div className="mt-1.5 flex items-center justify-between gap-2 text-xs font-bold text-ink-soft">
                <span>{level.level >= 15 ? t("regions.complete") : coach ? t("regions.next", { percent: formatPercent(level.nextPercent) }) : ""}</span>
                {stat.friendRank > 0 && (
                  <span className="inline-flex items-center gap-1">
                    <Trophy className="size-3.5" />
                    {t("regions.friendRank", { rank: stat.friendRank })}
                  </span>
                )}
              </div>
            </>
          );
          return (
            <li key={region.id}>
              {onPick ? (
                <button type="button" onClick={() => onPick(region.id)} className={cn("pressable sticker block w-full rounded-card bg-surface p-4")}>
                  {body}
                </button>
              ) : (
                <div className="sticker rounded-card bg-surface p-4">{body}</div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
