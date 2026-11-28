import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { quest, toMillis } from "@tokuly/api";
import { formatDay, questKeys } from "@tokuly/core/quest";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useAccountNow } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { leaderboardClient, useLocalized } from "./api";
import {
  BadgeMedal,
  LevelPill,
  PersonRow,
  QuestNav,
  SectionTitle,
  TitlePill,
  useBadgeName,
  useRemainingText,
} from "./parts";

const MEDALS = ["🥇", "🥈", "🥉"];

const PERIOD_KEY = {
  [quest.LeaderboardPeriod.UNSPECIFIED]: "allTime",
  [quest.LeaderboardPeriod.WEEK]: "week",
  [quest.LeaderboardPeriod.SEASON]: "season",
  [quest.LeaderboardPeriod.ALL_TIME]: "allTime",
} as const;

export interface RankingSearch {
  board?: string;
  period?: "week" | "season" | "allTime";
}

/**
 * `/quest/ranking`: the running season (a calendar month of XP among
 * friends, with its badge and countdown) and the boards — XP by week, season
 * or all time, and the longest streak. Only the player and their friends are
 * ever listed.
 */
export function RankingScreen({
  search,
  onSearch,
}: {
  search: RankingSearch;
  onSearch(search: RankingSearch): void;
}) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const boards = useQuery({
    queryKey: questKeys.boards,
    queryFn: async () => (await leaderboardClient().listLeaderboards({})).boards,
    staleTime: 10 * 60_000,
  });

  const board = boards.data?.find((item) => item.key === search.board) ?? boards.data?.[0];
  const period =
    board?.periods.find((item) => PERIOD_KEY[item] === search.period) ?? board?.periods[0];

  return (
    <Page title={t("ranking.title")} wide back="/quest">
      <QuestNav className="lg:max-w-xl" />
      <div className="grid gap-7 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)] lg:items-start">
        <div className="flex flex-col gap-7">
          <SeasonCard />
          <SeasonResults />
        </div>

        <section aria-label={t("ranking.boards")}>
          <SectionTitle>{t("ranking.boards")}</SectionTitle>
          {boards.isPending ? (
            <Skeleton className="mt-3 h-48" />
          ) : !board || period === undefined ? (
            <EmptyState emoji="🏝️" title={t("ranking.noBoards")} />
          ) : (
            <>
              <div className="mt-3 flex flex-wrap gap-2">
                <Tabs
                  items={(boards.data ?? []).map((item) => ({
                    value: item.key,
                    label: `${item.emoji} ${text(item.name)}`,
                  }))}
                  value={board.key}
                  onChange={(key) => onSearch({ board: key })}
                />
                {board.periods.length > 1 && (
                  <Tabs
                    items={board.periods.map((item) => ({
                      value: PERIOD_KEY[item],
                      label: t(`ranking.periods.${PERIOD_KEY[item]}`),
                    }))}
                    value={PERIOD_KEY[period]}
                    onChange={(value) => onSearch({ board: board.key, period: value })}
                  />
                )}
              </div>
              <Board board={board} period={period} />
            </>
          )}
        </section>
      </div>
    </Page>
  );
}

function Board({ board, period }: { board: quest.Leaderboard; period: quest.LeaderboardPeriod }) {
  const { t, i18n } = useTranslation("quest");
  const text = useLocalized();
  const { data, isPending, isError } = useQuery({
    queryKey: questKeys.leaderboard(board.key, period),
    queryFn: () => leaderboardClient().getLeaderboard({ board: board.key, period, limit: 100 }),
  });
  const unit = text(board.unit);
  const entries = data?.entries ?? [];
  const mine = data?.me;
  const mineListed = entries.some((entry) => entry.me);
  // Alone on the board is not a ranking yet.
  const lonely = !isPending && entries.filter((entry) => !entry.me).length === 0;

  return (
    <div className="sticker mt-3 rounded-card bg-surface p-2">
      {data?.periodStart && data.periodEnd && (
        <p className="px-3 pt-1.5 pb-1 text-xs font-bold text-ink-faint">
          {t("ranking.range", {
            from: formatDay(data.periodStart, i18n.language),
            to: formatDay(data.periodEnd, i18n.language),
          })}
        </p>
      )}
      {isPending ? (
        <div className="flex flex-col gap-2 p-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : isError ? (
        <EmptyState emoji="🛰️" title={t("ranking.error")} />
      ) : entries.length === 0 && !mine?.user ? (
        <EmptyState emoji="🏝️" title={t("ranking.empty")} body={t("ranking.emptyBody")} />
      ) : (
        <ol aria-label={text(board.name)}>
          {entries.map((entry) => (
            <li key={entry.user?.id ?? entry.rank}>
              <EntryRow entry={entry} unit={unit} />
            </li>
          ))}
          {mine?.user && !mineListed && (
            <>
              {entries.length > 0 && (
                <li className="px-4 py-1 text-center text-ink-faint" aria-hidden>
                  ⋯
                </li>
              )}
              <li>
                <EntryRow entry={mine} unit={unit} />
              </li>
            </>
          )}
        </ol>
      )}
      {lonely && !isError && (
        <div className="m-2 flex items-center gap-3 rounded-2xl bg-surface-2 p-3">
          <span className="text-2xl" aria-hidden>
            👯
          </span>
          <p className="min-w-0 flex-1 text-sm font-bold text-ink-soft">{t("ranking.lonely")}</p>
          <Link
            to="/friends"
            className="pressable sticker-sm inline-flex h-9 shrink-0 items-center rounded-full bg-lime px-4 text-sm font-black text-[#17122b]"
          >
            {t("ranking.addFriends")}
          </Link>
        </div>
      )}
    </div>
  );
}

function EntryRow({ entry, unit }: { entry: quest.LeaderboardEntry; unit: string }) {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const title = text(entry.title?.text);
  const ranked = entry.rank > 0;
  return (
    <PersonRow
      user={entry.user}
      highlighted={entry.me}
      label={t("ranking.rowLabel", {
        name: entry.user?.displayName ?? "",
        rank: entry.rank,
        value: Number(entry.value).toLocaleString(),
        unit,
      })}
      leading={
        <span className="w-8 shrink-0 text-center text-lg font-black tabular-nums">
          {ranked ? (MEDALS[entry.rank - 1] ?? entry.rank) : "–"}
        </span>
      }
      subtitle={
        <>
          <LevelPill level={entry.level} />
          {title && <TitlePill title={title} className="min-w-0 shrink" />}
          {entry.me && (
            <span className="rounded-full bg-brand px-1.5 py-0.5 text-[10px] font-black text-on-brand">
              {t("ranking.you")}
            </span>
          )}
        </>
      }
      trailing={
        <span className="shrink-0 text-right font-black tabular-nums">
          {ranked ? Number(entry.value).toLocaleString() : t("ranking.unranked")}
          {ranked && <span className="ml-0.5 text-xs font-bold text-ink-faint">{unit}</span>}
        </span>
      }
    />
  );
}

/** The season that is running: the badge to win, where the player stands, and how long is left. */
function SeasonCard() {
  const { t, i18n } = useTranslation("quest");
  const text = useLocalized();
  const badgeName = useBadgeName();
  const left = useRemainingText();
  const { ms: now } = useAccountNow(60_000);
  const { data: season, isPending } = useQuery({
    queryKey: questKeys.season,
    queryFn: async () => (await leaderboardClient().getSeason({})).season,
  });

  if (isPending) return <Skeleton className="h-52" />;
  if (!season) return null;
  const contest = season.badgeRank > 0;
  const winning = contest && season.rank > 0 && season.rank <= season.badgeRank;
  return (
    <section
      aria-label={t("season.title")}
      className="sticker overflow-hidden rounded-card bg-surface"
    >
      <div className="flex items-center gap-4 bg-brand p-5 text-on-brand">
        {season.badge && (
          <Link
            to="/quest/badges/$key"
            params={{ key: season.badge.key }}
            aria-label={badgeName(season.badge)}
            className="shrink-0"
          >
            <BadgeMedal badge={season.badge} owned size="lg" />
          </Link>
        )}
        <div className="min-w-0 flex-1">
          <p className="text-xs font-black opacity-80">{t("season.title")}</p>
          <h2 className="text-xl leading-tight font-black">{text(season.name)}</h2>
          <p className="mt-1 text-sm font-bold">
            {formatDay(season.startsOn, i18n.language)} – {formatDay(season.endsOn, i18n.language)}
          </p>
          {season.endsAt && (
            <p className="mt-1.5 inline-block rounded-full bg-surface px-2.5 py-0.5 text-xs font-black text-ink">
              {t("season.endsIn", { left: left(toMillis(season.endsAt), now) })}
            </p>
          )}
        </div>
      </div>
      <dl className="grid grid-cols-3 divide-x-2 divide-line border-t-2 border-ink text-center">
        <div className="flex flex-col-reverse px-2 py-3">
          <dt className="text-xs font-bold text-ink-faint">{t("season.xp")}</dt>
          <dd className="text-lg font-black tabular-nums">{Number(season.xp).toLocaleString()}</dd>
        </div>
        <div className="flex flex-col-reverse px-2 py-3">
          <dt className="text-xs font-bold text-ink-faint">{t("season.rank")}</dt>
          <dd className="text-lg font-black tabular-nums">
            {season.rank > 0
              ? t("season.rankOf", { rank: season.rank, size: season.groupSize })
              : "–"}
          </dd>
        </div>
        <div className="flex flex-col-reverse px-2 py-3">
          <dt className="text-xs font-bold text-ink-faint">{t("season.badgeLine")}</dt>
          <dd className="text-lg font-black tabular-nums">
            {contest ? t("season.topN", { rank: season.badgeRank }) : "–"}
          </dd>
        </div>
      </dl>
      <p
        className={cn(
          "border-t-2 border-line px-4 py-3 text-sm font-bold",
          winning ? "text-success" : "text-ink-soft",
        )}
      >
        {!contest
          ? t("season.noContest")
          : winning
            ? t("season.winning")
            : t("season.chasing", { rank: season.badgeRank })}
      </p>
    </section>
  );
}

/** How earlier seasons ended. */
function SeasonResults() {
  const { t } = useTranslation("quest");
  const text = useLocalized();
  const query = useInfiniteQuery({
    queryKey: questKeys.seasonResults,
    queryFn: ({ pageParam }) =>
      leaderboardClient().listSeasonResults({ page: { cursor: pageParam, limit: 12 } }),
    initialPageParam: "",
    getNextPageParam: (last) => (last.page?.hasMore ? last.page.nextCursor : undefined),
  });
  const seasons = query.data?.pages.flatMap((page) => page.seasons) ?? [];
  if (seasons.length === 0) return null;
  return (
    <section aria-label={t("season.past")}>
      <SectionTitle>{t("season.past")}</SectionTitle>
      <ul className="sticker mt-3 divide-y-2 divide-line rounded-card bg-surface px-4">
        {seasons.map((season) => (
          <li key={season.id} className="flex items-center gap-3 py-3">
            {season.badge && <BadgeMedal badge={season.badge} owned={season.badgeWon} size="sm" />}
            <span className="min-w-0 flex-1">
              <span className="block truncate font-black">{text(season.name)}</span>
              <span className="block text-xs font-bold text-ink-faint">
                {t("xp.amount", { xp: Number(season.xp).toLocaleString() })}
                {season.badgeWon && ` · ${t("season.won")}`}
              </span>
            </span>
            <span className="shrink-0 font-black tabular-nums">
              {season.rank > 0
                ? t("season.rankOf", { rank: season.rank, size: season.groupSize })
                : "–"}
            </span>
          </li>
        ))}
      </ul>
      <LoadMore query={query} />
    </section>
  );
}
