import { Link } from "@tanstack/react-router";
import { type common, type geo, type social, toMillis } from "@tokuly/api";
import { distanceTo, ghostRuleFor, isBlurred, seeEachOther } from "@tokuly/core/map";
import { Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { TextField } from "#/components/ui/field";
import { Tabs } from "#/components/ui/tabs";
import { FreeFriendsStrip } from "#/features/plans";
import { formatDistance } from "#/lib/format";
import { useLocations } from "#/stores/locations";
import { useFlyToUser } from "./camera";
import { friendName, useFriendsById, useGhostSettings, useNow } from "./data";
import { usePlaceText } from "./friend-sheet";
import { BatteryBadge, usePinCaption } from "./pins";
import { MapSheet } from "./sheet";
import { useMapUi } from "./ui-store";

interface Entry {
  friend: social.Friend;
  user: common.UserSummary;
  location: geo.FriendLocation | undefined;
  distance: number | null;
  recordedAt: number;
}

function FriendRow({
  entry,
  now,
  onSelect,
}: {
  entry: Entry & { location: geo.FriendLocation };
  now: number;
  onSelect(): void;
}) {
  const { t } = useTranslation("map");
  const online = useLocations((state) => state.online[entry.user.id]);
  const caption = usePinCaption(entry.location, now);
  const placeText = usePlaceText();
  const blurred = isBlurred(entry.location);
  return (
    <li>
      <button
        type="button"
        data-testid="friend-row"
        onClick={onSelect}
        className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left hover:bg-surface-2"
      >
        <Avatar user={entry.user} size="md" outlined online={online} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-bold">{friendName(entry.friend, entry.user)}</span>
          <span className="block truncate text-xs text-ink-soft">
            {[placeText(entry.location), caption.text].filter(Boolean).join(" · ")}
          </span>
        </span>
        {entry.location.batteryPercent !== undefined && (
          <BatteryBadge
            percent={entry.location.batteryPercent}
            charging={entry.location.charging}
            className="!shadow-none"
          />
        )}
        <span className="w-16 shrink-0 text-right text-xs font-black">
          {entry.distance !== null
            ? formatDistance(entry.distance)
            : blurred
              ? t("list.around")
              : "—"}
        </span>
      </button>
    </li>
  );
}

/** Everyone on the map as a list: nearest or most recent first, searchable; a tap flies to the person. */
export function FriendListPanel() {
  const { t } = useTranslation("map");
  const friends = useFriendsById();
  const locations = useLocations((state) => state.friends);
  const own = useLocations((state) => state.own);
  const { data: ghost } = useGhostSettings();
  const now = useNow(15_000);
  const flyToUser = useFlyToUser();
  const { stack, back, close, push } = useMapUi();
  const [sort, setSort] = useState<"distance" | "recent">("distance");
  const [search, setSearch] = useState("");

  const needle = search.trim().toLowerCase();
  const entries: Entry[] = Object.values(friends)
    .filter((friend): friend is social.Friend & { user: common.UserSummary } =>
      Boolean(friend.user),
    )
    .filter(
      (friend) =>
        !needle ||
        friendName(friend, friend.user).toLowerCase().includes(needle) ||
        friend.user.handle.toLowerCase().includes(needle),
    )
    .map((friend) => {
      const location = locations[friend.user.id]?.position ? locations[friend.user.id] : undefined;
      // A distance is only honest between two people who see each other precisely.
      const mutual = seeEachOther(location, ghostRuleFor(ghost, friend.user.id, now), now);
      return {
        friend,
        user: friend.user,
        location,
        distance: mutual ? distanceTo(own, location) : null,
        recordedAt: toMillis(location?.recordedAt),
      };
    });
  const located = entries
    .filter((entry): entry is Entry & { location: geo.FriendLocation } => Boolean(entry.location))
    .sort((a, b) =>
      sort === "distance"
        ? (a.distance ?? Number.POSITIVE_INFINITY) - (b.distance ?? Number.POSITIVE_INFINITY) ||
          b.recordedAt - a.recordedAt
        : b.recordedAt - a.recordedAt,
    );
  const hidden = entries.filter((entry) => !entry.location);
  const total = Object.keys(friends).length;

  return (
    <MapSheet
      testId="friend-list"
      title={t("list.title")}
      subtitle={t("list.count", { shown: located.length, total })}
      onBack={stack.length > 1 ? back : undefined}
      onClose={stack.length > 0 ? close : undefined}
    >
      {total === 0 ? (
        <div className="flex flex-col items-center gap-2 py-8 text-center">
          <span className="animate-float text-5xl">🫡</span>
          <p className="font-black text-balance">{t("noFriendsTitle")}</p>
          <p className="max-w-64 text-sm text-balance text-ink-soft">{t("noFriendsBody")}</p>
          <Link
            to="/friends"
            className="pressable sticker-sm mt-2 flex h-11 items-center justify-center rounded-full bg-lime px-6 text-sm font-black text-[#17122b]"
          >
            {t("addFriends")}
          </Link>
        </div>
      ) : (
        <>
          {/* Who is free right now, and "ひま!" for oneself. */}
          <FreeFriendsStrip bare quiet className="pb-3" />
          <div className="space-y-2 pb-2">
            <TextField
              aria-label={t("list.search")}
              placeholder={t("list.search")}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              trailing={<Search className="mr-3 size-4 text-ink-faint" />}
            />
            <Tabs<"distance" | "recent">
              items={[
                { value: "distance", label: t("list.byDistance") },
                { value: "recent", label: t("list.byRecent") },
              ]}
              value={sort}
              onChange={setSort}
            />
          </div>
          <ul className="space-y-0.5">
            {located.map((entry) => (
              <FriendRow
                key={entry.user.id}
                entry={entry}
                now={now}
                onSelect={() => {
                  push({ kind: "friend", userId: entry.user.id });
                  flyToUser(entry.user.id);
                }}
              />
            ))}
          </ul>
          {located.length === 0 && hidden.length === 0 && (
            <p className="py-6 text-center text-sm text-ink-soft">{t("list.noMatch")}</p>
          )}
          {hidden.length > 0 && (
            <>
              <p className="mt-4 mb-1 px-2 text-xs font-black text-ink-soft">
                {t("list.notOnMap")}
              </p>
              <ul className="space-y-0.5">
                {hidden.map((entry) => (
                  <li key={entry.user.id}>
                    <button
                      type="button"
                      onClick={() => push({ kind: "friend", userId: entry.user.id })}
                      className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left opacity-70 hover:bg-surface-2"
                    >
                      <Avatar user={entry.user} size="md" />
                      <span className="min-w-0 flex-1 truncate font-bold">
                        {friendName(entry.friend, entry.user)}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </>
      )}
    </MapSheet>
  );
}
