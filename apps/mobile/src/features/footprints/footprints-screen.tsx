import { geo, toDate } from "@tokuly/api";
import { explorerLevel, formatPercent, regionLevel, regionName } from "@tokuly/core/footprints";
import { formatDistance } from "@tokuly/core/format";
import { router } from "expo-router";
import { Map as MapIcon } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, ErrorNote, Page, Row, Section, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { useAccountTimeZone } from "#/features/history/data";
import { GazetteerCredit } from "#/features/history/parts";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { NS, useFootprintStats, useLeaderboard, useMarkRecapSeen, useRecap } from "./data";

const MEDALS = ["🥇", "🥈", "🥉"];

/** The flag of a country code as an emoji. */
const flagEmoji = (code: string) =>
  /^[A-Za-z]{2}$/.test(code)
    ? String.fromCodePoint(
        ...[...code.toUpperCase()].map((letter) => 0x1f1e6 + letter.charCodeAt(0) - 65),
      )
    : "🌏";

/**
 * `/footprints`: the account's own scratch map as a game — how much it has
 * uncovered, its explorer level, the regions and their levels, what is new
 * since it last looked, and the ranking. The map itself is on the web.
 */
export function FootprintsScreen() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const stats = useFootprintStats();
  if (stats.isPending) return <Spinner />;
  if (stats.isError)
    return (
      <Page testID="footprints">
        <ErrorNote>{errorMessage(stats.error, { namespace: NS })}</ErrorNote>
      </Page>
    );
  const total = Number(stats.data.totalCellCount);
  const regions = stats.data.regions;
  return (
    <Page testID="footprints">
      {total === 0 ? (
        <EmptyState
          emoji="👣"
          title={t("empty.title")}
          body={t("empty.body")}
          action={
            <Button variant="lime" onPress={() => router.navigate("/map")}>
              {t("empty.action")}
            </Button>
          }
        />
      ) : (
        <Hero stats={stats.data} />
      )}
      <RecapCard />
      <Section>
        <Row
          testID="footprints-map"
          icon={<MapIcon size={18} color={colors.ink} />}
          label={t("mobile:footprints.mapOnWeb")}
          onPress={() => router.push(webScreenHref("/footprints") as never)}
        />
      </Section>
      <RegionList stats={regions} />
      <Leaderboard regions={regions.flatMap((stat) => (stat.region ? [stat.region] : []))} />
      <Text className="text-center text-xs text-ink-faint">{t("privacyNote")}</Text>
      {regions.length > 0 && <GazetteerCredit />}
    </Page>
  );
}

function Meter({ value, label }: { value: number; label: string }) {
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      accessibilityValue={{ min: 0, max: 100, now: Math.round(value * 100) }}
      className="h-3 overflow-hidden rounded-full bg-surface-3"
    >
      <View
        className="h-full rounded-full bg-lime"
        style={{ width: `${Math.max(2, Math.min(100, value * 100))}%` }}
      />
    </View>
  );
}

function Hero({ stats }: { stats: geo.GetFootprintStatsResponse }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const total = Number(stats.totalCellCount);
  const explorer = explorerLevel(total);
  const top =
    stats.regions.find((stat) => stat.region?.kind === geo.RegionKind.ADMIN1) ?? stats.regions[0];
  const km2 = stats.world?.exploredKm2 ?? 0;
  return (
    <View
      testID="footprints-hero"
      className="gap-4 rounded-card bg-surface p-5"
      style={sticker(colors)}
    >
      {top?.region && (
        <Text className="text-2xl font-black text-ink">
          {t("hero.headline", {
            region: regionName(top.region),
            percent: formatPercent(top.percent),
          })}
        </Text>
      )}
      <View className="flex-row items-center gap-4">
        <View
          testID="footprints-level"
          className="size-16 items-center justify-center rounded-2xl bg-lime"
          style={sticker(colors)}
        >
          <Text className="text-[10px] font-black" style={{ color: INK_ON_BRIGHT }}>
            Lv.
          </Text>
          <Text className="text-2xl font-black" style={{ color: INK_ON_BRIGHT }}>
            {explorer.level}
          </Text>
        </View>
        <View className="min-w-0 flex-1 gap-1.5">
          <Text numberOfLines={1} className="font-black text-ink">
            {t(`titles.${explorer.title}`)}
          </Text>
          <Meter value={explorer.progress} label={t("hero.levelProgress")} />
          <Text className="text-xs font-bold text-ink-soft">
            {t("hero.toNext", { count: explorer.next - total, level: explorer.level + 1 })}
          </Text>
        </View>
      </View>
      <View className="flex-row gap-2">
        <Fact label={t("hero.cells")} value={total.toLocaleString()} />
        <Fact label={t("hero.countries")} value={String(stats.countryCount)} />
        <Fact label={t("hero.area")} value={`${km2.toFixed(km2 < 100 ? 1 : 0)} km²`} />
      </View>
    </View>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <View className="flex-1 items-center rounded-2xl bg-surface-2 px-2 py-2.5">
      <Text numberOfLines={1} className="text-lg font-black text-ink">
        {value}
      </Text>
      <Text className="text-[11px] font-bold text-ink-faint">{label}</Text>
    </View>
  );
}

/** What was uncovered since the account last looked; "見たよ" starts the next one from now. */
function RecapCard() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const zone = useAccountTimeZone();
  const recap = useRecap();
  const seen = useMarkRecapSeen();
  const count = Number(recap.data?.newCellCount ?? 0);
  if (!recap.data || count === 0) return null;
  const since = toDate(recap.data.since);
  return (
    <View
      testID="footprints-recap"
      className="gap-2 rounded-card bg-sun p-4"
      style={sticker(colors)}
    >
      <Text className="text-xs font-black" style={{ color: INK_ON_BRIGHT }}>
        🎁 {t("recap.title")}
        {since
          ? `・${t("recap.since", { date: since.toLocaleDateString(i18n.language, { timeZone: zone, month: "short", day: "numeric" }) })}`
          : ""}
      </Text>
      <Text className="text-2xl font-black" style={{ color: INK_ON_BRIGHT }}>
        {t("recap.cardHeadline", { count })}
      </Text>
      {recap.data.distanceMeters > 0 && (
        <Text className="font-bold" style={{ color: INK_ON_BRIGHT }}>
          👟 {t("recap.distance", { distance: formatDistance(recap.data.distanceMeters) })}
        </Text>
      )}
      {recap.data.newRegions.map((region) => (
        <Text key={region.id} className="font-bold" style={{ color: INK_ON_BRIGHT }}>
          {flagEmoji(region.countryCode)} {t("recap.newRegion", { region: regionName(region) })}
        </Text>
      ))}
      <View className="items-end">
        <Button
          testID="footprints-recap-seen"
          variant="secondary"
          size="sm"
          loading={seen.isPending}
          onPress={() => seen.mutate()}
        >
          {t("mobile:footprints.recapSeen")}
        </Button>
      </View>
    </View>
  );
}

type Kind = "admin1" | "country";

/** Percent explored per prefecture and per country, each with its level and the way to the next. */
function RegionList({ stats }: { stats: geo.RegionStat[] }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const [kind, setKind] = useState<Kind>("admin1");
  const admin1 = stats.filter((stat) => stat.region?.kind === geo.RegionKind.ADMIN1);
  const countries = stats.filter((stat) => stat.region?.kind === geo.RegionKind.COUNTRY);
  const shown = kind === "admin1" && admin1.length > 0 ? admin1 : countries;
  if (stats.length === 0) return null;
  return (
    <View testID="footprints-regions" className="gap-3">
      <Text className="px-1 text-lg font-black text-ink">{t("regions.title")}</Text>
      {admin1.length > 0 && countries.length > 0 && (
        <Tabs<Kind>
          testID="footprints-regions-kind"
          items={[
            { value: "admin1", label: t("regions.admin1") },
            { value: "country", label: t("regions.country") },
          ]}
          value={kind}
          onChange={setKind}
        />
      )}
      {shown.map((stat) => {
        const region = stat.region;
        if (!region) return null;
        const level = regionLevel(stat.percent);
        return (
          <View
            key={region.id}
            className="gap-3 rounded-card bg-surface p-4"
            style={sticker(colors)}
          >
            <View className="flex-row items-center gap-3">
              <Text className="text-3xl">{flagEmoji(region.countryCode)}</Text>
              <View className="min-w-0 flex-1">
                <Text numberOfLines={1} className="font-black text-ink">
                  {regionName(region)}
                </Text>
                <Text className="text-xs font-bold text-ink-faint">
                  {t("regions.cells", {
                    count: Number(stat.cellCount),
                    km2: stat.exploredKm2.toFixed(stat.exploredKm2 < 10 ? 1 : 0),
                  })}
                </Text>
              </View>
              <View className="items-end">
                <Text className="text-xl font-black text-ink">{formatPercent(stat.percent)}</Text>
                <Text className="overflow-hidden rounded-full bg-brand-soft px-2 text-xs font-black text-brand-strong">
                  Lv.{level.level}
                </Text>
              </View>
            </View>
            <Meter
              value={level.progress}
              label={t("regions.progressLabel", { region: regionName(region) })}
            />
            <Text className="text-xs font-bold text-ink-soft">
              {level.progress >= 1 && level.nextPercent === 100
                ? t("regions.complete")
                : t("regions.next", { percent: formatPercent(level.nextPercent) })}
              {stat.friendRank > 0 ? `・${t("regions.friendRank", { rank: stat.friendRank })}` : ""}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

type Scope = "friends" | "global";

/** The ranking among friends or everyone who shows their footprints, by region. */
function Leaderboard({ regions }: { regions: geo.Region[] }) {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const me = useMe();
  const [regionId, setRegionId] = useState("world");
  const [scope, setScope] = useState<Scope>("friends");
  const board = useLeaderboard(
    regionId,
    scope === "friends" ? geo.LeaderboardScope.FRIENDS : geo.LeaderboardScope.GLOBAL,
  );
  const entries = board.data?.entries ?? [];
  const mine = board.data?.me;
  const mineListed = mine?.user && entries.some((entry) => entry.user?.id === mine.user?.id);
  const choices = [{ id: "world", name: t("leaderboard.world") }].concat(
    regions.slice(0, 6).map((region) => ({ id: region.id, name: regionName(region) })),
  );
  return (
    <View testID="footprints-leaderboard" className="gap-3">
      <Text className="px-1 text-lg font-black text-ink">{t("leaderboard.title")}</Text>
      <Tabs<Scope>
        testID="footprints-scope"
        items={[
          { value: "friends", label: t("leaderboard.friends") },
          { value: "global", label: t("leaderboard.global") },
        ]}
        value={scope}
        onChange={setScope}
      />
      <View className="flex-row flex-wrap gap-2">
        {choices.map((choice) => (
          <Pressable
            key={choice.id}
            testID={`footprints-board-${choice.id}`}
            accessibilityRole="radio"
            accessibilityState={{ selected: choice.id === regionId }}
            onPress={() => setRegionId(choice.id)}
            className={`h-8 justify-center rounded-full px-3 ${choice.id === regionId ? "bg-ink" : "bg-surface"}`}
          >
            <Text
              className={`text-sm font-black ${choice.id === regionId ? "text-surface" : "text-ink"}`}
            >
              {choice.name}
            </Text>
          </Pressable>
        ))}
      </View>
      <View className="rounded-card bg-surface p-2" style={sticker(colors)}>
        {board.isPending ? (
          <Spinner />
        ) : board.isError ? (
          <ErrorNote>{errorMessage(board.error, { namespace: NS })}</ErrorNote>
        ) : entries.length === 0 ? (
          <EmptyState emoji="🏝️" title={t("leaderboard.empty")} body={t("leaderboard.emptyBody")} />
        ) : (
          <>
            {entries.map((entry) => (
              <BoardRow
                key={entry.user?.id ?? entry.rank}
                entry={entry}
                self={entry.user?.id === me?.user?.id}
              />
            ))}
            {mine?.user && !mineListed && (
              <>
                <Text className="text-center text-ink-faint">⋯</Text>
                <BoardRow entry={mine} self />
              </>
            )}
          </>
        )}
        {board.isSuccess && scope === "friends" && entries.length <= 1 && (
          <View className="m-2 flex-row items-center gap-3 rounded-2xl bg-surface-2 p-3">
            <Text className="text-2xl">👯</Text>
            <Text className="flex-1 text-sm font-bold text-ink-soft">
              {t("leaderboard.lonely")}
            </Text>
            <Button size="sm" variant="lime" onPress={() => router.push("/add-friend")}>
              {t("leaderboard.addFriends")}
            </Button>
          </View>
        )}
      </View>
    </View>
  );
}

function BoardRow({ entry, self }: { entry: geo.LeaderboardEntry; self: boolean }) {
  const { t } = useTranslation(NS);
  const user = entry.user;
  if (!user) return null;
  return (
    <Pressable
      testID={`footprints-rank-${user.id}`}
      accessibilityRole="button"
      onPress={() => router.push(`/u/${user.handle}`)}
      className={`flex-row items-center gap-3 rounded-2xl px-2 py-2 ${self ? "bg-brand-soft" : ""}`}
    >
      <Text className="w-8 text-center text-lg font-black text-ink">
        {MEDALS[entry.rank - 1] ?? entry.rank}
      </Text>
      <Avatar user={user} size="md" />
      <View className="min-w-0 flex-1">
        <Text numberOfLines={1} className="font-bold text-ink">
          {user.displayName}
          {self ? ` (${t("leaderboard.you")})` : ""}
        </Text>
        <Text className="text-xs font-bold text-ink-faint">
          {t("leaderboard.cells", { count: Number(entry.cellCount) })}
        </Text>
      </View>
      <Text className="font-black text-ink">{formatPercent(entry.percent)}</Text>
    </Pressable>
  );
}
