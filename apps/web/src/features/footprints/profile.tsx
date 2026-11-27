import { Link } from "@tanstack/react-router";
import { formatPercent } from "@tokuly/core/footprints";
import { useTranslation } from "react-i18next";
import { type ProfileSection, type ProfileSectionProps, ProfileStat } from "#/seams/profile";

/**
 * The footprint numbers of a profile. They lead to the scratch map they come
 * from: my own, or as much of theirs as they show.
 */
function FootprintStats({ profile, user, isSelf }: ProfileSectionProps) {
  const { t } = useTranslation("footprints");
  const stats = profile.stats;
  if (!stats) return null;
  const search = isSelf ? {} : { user: user.id };
  const cell = "rounded-2xl transition-colors hover:bg-surface-2";
  return (
    <>
      <Link
        to="/footprints"
        search={search}
        className={cell}
        aria-label={t("profile.explored", { percent: formatPercent(stats.exploredPercent) })}
      >
        <ProfileStat
          value={formatPercent(stats.exploredPercent)}
          label={t("profile:stats.explored")}
        />
      </Link>
      <Link
        to="/footprints"
        search={search}
        className={cell}
        aria-label={t("profile.countries", { count: stats.visitedCountryCount })}
      >
        <ProfileStat value={stats.visitedCountryCount} label={t("profile:stats.countries")} />
      </Link>
    </>
  );
}

export default [
  { id: "footprints", slot: "stat", order: 10, Component: FootprintStats },
] satisfies ProfileSection[];
