import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { geo } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { footprintClient, footprintKeys, regionName } from "./api";
import { formatPercent } from "./levels";

const MEDALS = ["🥇", "🥈", "🥉"];
type Scope = "friends" | "global";

/** Who has explored the most of a region: among friends, or everyone who shows their map. */
export function Leaderboard({
  regions,
  regionId,
  onRegion,
  scope,
  onScope,
}: {
  /** Regions to choose from (the caller's own); the world is always offered. */
  regions: geo.Region[];
  regionId: string;
  onRegion(regionId: string): void;
  scope: Scope;
  onScope(scope: Scope): void;
}) {
  const { t } = useTranslation("footprints");
  const me = useMe();
  const scopeValue =
    scope === "friends" ? geo.LeaderboardScope.FRIENDS : geo.LeaderboardScope.GLOBAL;
  const { data, isPending, isError } = useQuery({
    queryKey: footprintKeys.leaderboard(regionId, scopeValue),
    queryFn: () =>
      footprintClient().getLeaderboard({
        regionId: regionId === "world" ? "" : regionId,
        scope: scopeValue,
        limit: 50,
      }),
  });

  const entries = data?.entries ?? [];
  const mine = data?.me;
  const mineListed = mine?.user && entries.some((entry) => entry.user?.id === mine.user?.id);
  // Alone on the board is not a ranking yet.
  const lonely = scope === "friends" && entries.length <= 1;

  return (
    <section aria-label={t("leaderboard.title")}>
      <h2 className="text-lg font-black">{t("leaderboard.title")}</h2>
      <div className="mt-3 flex flex-col gap-3 sm:flex-row sm:items-end">
        <SelectField
          className="sm:flex-1"
          label={t("leaderboard.region")}
          value={regionId}
          onChange={onRegion}
          options={[
            { value: "world", label: t("leaderboard.world") },
            ...regions.map((region) => ({ value: region.id, label: regionName(region) })),
          ]}
        />
        <Tabs
          items={[
            { value: "friends", label: t("leaderboard.friends") },
            { value: "global", label: t("leaderboard.global") },
          ]}
          value={scope}
          onChange={onScope}
        />
      </div>

      <div className="sticker mt-3 rounded-card bg-surface p-2">
        {isPending ? (
          <div className="flex flex-col gap-2 p-2">
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
          </div>
        ) : isError || entries.length === 0 ? (
          <EmptyState emoji="🏝️" title={t("leaderboard.empty")} body={t("leaderboard.emptyBody")} />
        ) : (
          <ol>
            {entries.map((entry) => (
              <Row
                key={entry.user?.id ?? entry.rank}
                entry={entry}
                self={entry.user?.id === me?.user?.id}
              />
            ))}
            {mine?.user && !mineListed && (
              <>
                <li className="px-4 py-1 text-center text-ink-faint" aria-hidden>
                  ⋯
                </li>
                <Row entry={mine} self />
              </>
            )}
          </ol>
        )}
        {!isPending && lonely && (
          <div className="m-2 flex items-center gap-3 rounded-2xl bg-surface-2 p-3">
            <span className="text-2xl" aria-hidden>
              👯
            </span>
            <p className="min-w-0 flex-1 text-sm font-bold text-ink-soft">
              {t("leaderboard.lonely")}
            </p>
            <Link
              to="/friends"
              className="pressable sticker-sm inline-flex h-9 shrink-0 items-center rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
            >
              {t("leaderboard.addFriends")}
            </Link>
          </div>
        )}
      </div>
    </section>
  );
}

function Row({ entry, self }: { entry: geo.LeaderboardEntry; self: boolean }) {
  const { t } = useTranslation("footprints");
  const user = entry.user;
  if (!user) return null;
  const body = (
    <>
      <span className="w-8 shrink-0 text-center text-lg font-black tabular-nums">
        {MEDALS[entry.rank - 1] ?? entry.rank}
      </span>
      <Avatar user={user} size="md" />
      <span className="min-w-0 flex-1 text-left">
        <span className="block truncate font-bold">
          <EmojiText text={user.displayName} />
          {self && (
            <span className="ml-1.5 rounded-full bg-brand px-1.5 py-0.5 text-[10px] font-black text-on-brand">
              {t("leaderboard.you")}
            </span>
          )}
        </span>
        <span className="block truncate text-xs font-bold text-ink-faint">
          {t("leaderboard.cells", { count: Number(entry.cellCount) })}
        </span>
      </span>
      <span className="shrink-0 font-black tabular-nums">{formatPercent(entry.percent)}</span>
    </>
  );
  const classes = cn(
    "flex w-full items-center gap-3 rounded-2xl px-3 py-2.5",
    self ? "bg-brand-soft" : "transition-colors hover:bg-surface-2",
  );
  return (
    <li>
      {self ? (
        <div className={classes}>{body}</div>
      ) : (
        <Link
          to="/footprints"
          search={{ user: user.id }}
          className={classes}
          aria-label={t("leaderboard.open", { name: user.displayName })}
        >
          {body}
        </Link>
      )}
    </li>
  );
}
