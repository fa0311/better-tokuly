import { spaces, toMillis } from "@tokuly/api";
import { compactCount } from "@tokuly/core/engagement";
import { insideOf, nameOf } from "@tokuly/core/spaces";
import { router } from "expo-router";
import { Headphones, Mic, Radio, Users } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import i18n from "#/lib/i18n";
import { sticker, useTheme } from "#/lib/theme";
import { useAudioSession } from "./session";

/** "今日 18:00", "あした 9:30", "12/3(木) 20:00": when a room is planned for, on the phone's clock. */
export function formatWhen(ms: number, now = Date.now()): string {
  const date = new Date(ms);
  const time = `${date.getHours()}:${String(date.getMinutes()).padStart(2, "0")}`;
  const day = (offset: number) => {
    const other = new Date(now);
    other.setDate(other.getDate() + offset);
    return other.toDateString() === date.toDateString();
  };
  if (day(0)) return i18n.t("spaces:when.today", { time });
  if (day(1)) return i18n.t("spaces:when.tomorrow", { time });
  const weekday = date.toLocaleDateString(i18n.language, {
    month: "numeric",
    day: "numeric",
    weekday: "short",
  });
  return `${weekday} ${time}`;
}

/** The red mark of a room that is on now. */
export function LiveMark() {
  const { t } = useTranslation("spaces");
  return (
    <View className="flex-row items-center gap-1 self-start rounded-md bg-live px-1.5 py-0.5">
      <Radio size={11} color="#ffffff" />
      <Text className="text-[11px] font-black text-white">{t("live")}</Text>
    </View>
  );
}

/** A row of faces, overlapping a little. */
export function Faces({
  users,
  max = 4,
}: {
  users: spaces.SpaceParticipant["user"][];
  max?: number;
}) {
  return (
    <View className="flex-row">
      {users.slice(0, max).map((user, index) => (
        <View key={user?.id ?? index} style={{ marginLeft: index === 0 ? 0 : -8 }}>
          <Avatar user={user} size="sm" outlined />
        </View>
      ))}
    </View>
  );
}

/** A room on a list: live or planned, who is on stage, which friends are inside. */
export function SpaceCard({ space, testID }: { space: spaces.Space; testID?: string }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const live = space.state === spaces.SpaceState.LIVE;
  const friend = space.friendsInside[0];
  return (
    <Pressable
      testID={testID ?? "space-card"}
      accessibilityRole="button"
      accessibilityLabel={space.title}
      onPress={() => router.push(`/spaces/${space.id}`)}
      className="gap-3 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      <View className="flex-row items-center gap-2">
        {live ? (
          <LiveMark />
        ) : (
          <Text className="text-xs font-black text-brand">
            {formatWhen(toMillis(space.scheduledAt))}
          </Text>
        )}
        <Text className="text-xs font-bold text-ink-soft">
          {t(
            `audience.${space.audience === spaces.SpaceAudience.PUBLIC ? "public" : space.audience === spaces.SpaceAudience.INVITED ? "invited" : "friends"}`,
          )}
        </Text>
      </View>
      <Text numberOfLines={2} className="text-lg leading-6 font-black text-ink">
        {space.title}
      </Text>
      {space.tags.length > 0 && (
        <Text numberOfLines={1} className="text-xs font-bold text-brand">
          {space.tags.map((tag) => `#${tag}`).join("  ")}
        </Text>
      )}
      <View className="flex-row items-center gap-3">
        <Faces users={space.stage.length ? space.stage.map((p) => p.user) : [space.host]} />
        <Text numberOfLines={1} className="flex-1 text-sm font-bold text-ink-soft">
          {nameOf(space.host)}
        </Text>
        {live ? (
          <View className="flex-row items-center gap-1">
            <Headphones size={14} color={colors["ink-soft"]} />
            <Text className="text-xs font-black text-ink-soft">
              {compactCount(space.listenerCount, i18n.language)}
            </Text>
          </View>
        ) : space.reminderCount > 0 ? (
          <Text className="text-xs font-bold text-ink-soft">
            {t("scheduled.reminders", { count: space.reminderCount })}
          </Text>
        ) : null}
      </View>
      {friend && (
        <Text className="rounded-full bg-lime/40 px-3 py-1 text-xs font-black text-ink self-start">
          {space.friendsInside.length > 1
            ? t("card.friendsInside", {
                name: nameOf(friend),
                count: space.friendsInside.length,
              })
            : t("card.friendInside", { name: nameOf(friend) })}
        </Text>
      )}
    </Pressable>
  );
}

/** A hangout on the board: its emoji, who is inside now. */
export function HangoutCard({ hangout }: { hangout: spaces.Hangout }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const inside = insideOf(hangout);
  const open = hangout.insideCount > 0;
  return (
    <Pressable
      testID="hangout-card"
      accessibilityRole="button"
      accessibilityLabel={hangout.name}
      onPress={() => router.push(`/spaces/hangouts/${hangout.id}`)}
      className="flex-row items-center gap-3 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      <View
        className={`size-14 items-center justify-center rounded-2xl ${open ? "bg-lime" : "bg-surface-2"}`}
      >
        <Text className="text-3xl">{hangout.emoji || "🏠"}</Text>
      </View>
      <View className="flex-1 gap-1">
        <Text numberOfLines={1} className="text-base font-black text-ink">
          {hangout.name}
        </Text>
        <Text className={`text-xs font-bold ${open ? "text-success" : "text-ink-soft"}`}>
          {open
            ? t("hangouts.inside", { count: hangout.insideCount })
            : t("hangouts.nobody", { count: hangout.memberCount })}
        </Text>
      </View>
      {inside.length > 0 && <Faces users={inside.map((member) => member.user)} max={3} />}
    </Pressable>
  );
}

/**
 * The venue the person is inside, while they look at something else: a tap
 * goes back to it. Nothing while they are outside.
 */
export function InsideBar({ hereKind, hereId }: { hereKind?: string; hereId?: string }) {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  const active = useAudioSession((state) => state.active);
  if (!active || (active.kind === hereKind && active.id === hereId)) return null;
  const href = active.kind === "space" ? `/spaces/${active.id}` : `/spaces/hangouts/${active.id}`;
  return (
    <Pressable
      testID="spaces-inside-bar"
      accessibilityRole="button"
      onPress={() => router.push(href as never)}
      className="mx-4 mb-2 flex-row items-center gap-3 rounded-full bg-brand px-4 py-2.5"
      style={sticker(colors, "sm")}
    >
      {active.micOn ? (
        <Mic size={16} color={colors["on-brand"]} />
      ) : active.kind === "hangout" ? (
        <Users size={16} color={colors["on-brand"]} />
      ) : (
        <Headphones size={16} color={colors["on-brand"]} />
      )}
      <Text numberOfLines={1} className="flex-1 text-sm font-black text-on-brand">
        {t("spaces.insideNow", { title: `${active.emoji} ${active.title}`.trim() })}
      </Text>
    </Pressable>
  );
}
