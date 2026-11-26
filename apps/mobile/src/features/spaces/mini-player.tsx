import { useQueryClient } from "@tanstack/react-query";
import { router, usePathname } from "expo-router";
import { LogOut, Mic, MicOff } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { sticker, useTheme } from "#/lib/theme";
import {
  type ActiveVenue,
  leaveVenue,
  toggleMicrophone,
  useAudioSession,
  useVoices,
} from "./session";

/** The screen of a venue: where the full view of what is playing lives. */
function pathOf(venue: Pick<ActiveVenue, "kind" | "id">): string {
  return venue.kind === "space" ? `/spaces/${venue.id}` : `/spaces/hangouts/${venue.id}`;
}

/** Above the tab bar and the bars at the bottom of screens (a composer, a room's controls). */
const ABOVE_BOTTOM_BARS = 76;

/**
 * The room or hangout the person is inside, as a pill over every other
 * screen: what is playing, whether the microphone is open, mute and leave.
 * The venue's own screen is the full player, so it is not shown there.
 */
export function MiniPlayer() {
  const active = useAudioSession((state) => state.active);
  const pathname = usePathname();
  if (!active || pathname.replace(/\/+$/, "") === pathOf(active)) return null;
  return <Pill active={active} />;
}

function Pill({ active }: { active: ActiveVenue }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const queryClient = useQueryClient();
  const voices = useVoices(active.kind, active.id);
  const status = active.micOn
    ? "room.micOn"
    : active.phase === "connecting"
      ? "room.connecting"
      : active.phase === "reconnecting"
        ? "mini.reconnecting"
        : voices.speaking.size > 0
          ? "mini.talking"
          : "mini.listening";
  return (
    <View
      testID="spaces-mini-player"
      className="absolute left-3 flex-row items-center gap-1.5 self-start rounded-full bg-ink py-1.5 pr-1.5 pl-3"
      style={[
        { bottom: insets.bottom + ABOVE_BOTTOM_BARS, maxWidth: "90%", zIndex: 40, elevation: 8 },
        sticker(colors, "sm"),
      ]}
    >
      <Pressable
        testID="spaces-inside-bar"
        accessibilityRole="button"
        accessibilityLabel={t("mini.open", { title: active.title })}
        onPress={() => router.push(pathOf(active) as never)}
        className="shrink flex-row items-center gap-2"
      >
        {active.emoji ? (
          <Text>{active.emoji}</Text>
        ) : (
          <View className="size-2.5 rounded-full bg-live" />
        )}
        <View className="shrink">
          <Text numberOfLines={1} className="text-sm font-black text-bg">
            {active.title}
          </Text>
          <Text
            className={`text-[11px] font-bold ${active.micOn ? "text-lime" : "text-bg opacity-70"}`}
          >
            {t(status)}
          </Text>
        </View>
      </Pressable>
      {active.canPublish && (
        <Pressable
          testID="spaces-inside-bar-mic"
          accessibilityRole="button"
          accessibilityLabel={t(active.micOn ? "room.mute" : "room.unmute")}
          accessibilityState={{ selected: active.micOn }}
          disabled={active.phase !== "connected"}
          onPress={() => void toggleMicrophone(queryClient)}
          hitSlop={4}
          className={`size-9 items-center justify-center rounded-full ${active.micOn ? "bg-lime" : "bg-surface"}`}
        >
          {active.micOn ? (
            <Mic size={16} color="#17122b" />
          ) : (
            <MicOff size={16} color={colors.ink} />
          )}
        </Pressable>
      )}
      <Pressable
        testID="spaces-inside-bar-leave"
        accessibilityRole="button"
        accessibilityLabel={t("mini.leave")}
        onPress={() => void leaveVenue(queryClient)}
        hitSlop={4}
        className="size-9 items-center justify-center rounded-full bg-danger"
      >
        <LogOut size={16} color="#ffffff" />
      </Pressable>
    </View>
  );
}
