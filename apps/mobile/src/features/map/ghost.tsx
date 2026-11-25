import { useMutation, useQueryClient } from "@tanstack/react-query";
import { geo, toMillis } from "@tokuly/api";
import { compactDuration } from "@tokuly/core/format";
import { ghostRuleFor, ghostSummary, mapKeys } from "@tokuly/core/map";
import type { TFunction } from "i18next";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { locationClient, useGhostSettings } from "./data";

export const GHOST_MODES = [
  { mode: geo.GhostMode.PRECISE, key: "precise", emoji: "📍" },
  { mode: geo.GhostMode.BLURRED, key: "blurred", emoji: "🌫️" },
  { mode: geo.GhostMode.FROZEN, key: "frozen", emoji: "🧊" },
] as const;

const TIMERS = [
  { seconds: 0, key: "forever" },
  { seconds: 60 * 60, key: "h1" },
  { seconds: 4 * 60 * 60, key: "h4" },
  { seconds: 8 * 60 * 60, key: "h8" },
  { seconds: 24 * 60 * 60, key: "h24" },
] as const;

const MODE_KEY: Record<number, "precise" | "blurred" | "frozen"> = {
  [geo.GhostMode.PRECISE]: "precise",
  [geo.GhostMode.BLURRED]: "blurred",
  [geo.GhostMode.FROZEN]: "frozen",
};

/** Sets ghost mode for everyone (`userIds` empty) or for some friends. */
export function useSetGhost() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { userIds: string[]; mode: geo.GhostMode; durationSeconds: number }) =>
      locationClient().setGhostMode(input),
    onSuccess: (response) =>
      response.settings && queryClient.setQueryData(mapKeys.ghost, response.settings),
    onError: (error) => {
      Alert.alert(errorMessage(error, { namespace: "map" }));
      void queryClient.invalidateQueries({ queryKey: mapKeys.ghost });
    },
  });
}

/** Three-way switch: precise, blurred, frozen. */
export function GhostPick({
  current,
  onPick,
  label,
}: {
  current: geo.GhostMode;
  onPick(mode: geo.GhostMode): void;
  label: string;
}) {
  const { t } = useTranslation("map");
  const { colors } = useTheme();
  return (
    <View accessibilityRole="radiogroup" accessibilityLabel={label} className="flex-row gap-2">
      {GHOST_MODES.map(({ mode, key, emoji }) => {
        const on = current === mode;
        return (
          <Pressable
            key={key}
            testID={`ghost-${key}`}
            accessibilityRole="radio"
            accessibilityState={{ checked: on }}
            onPress={() => !on && onPick(mode)}
            className={`flex-1 items-center gap-1 rounded-control py-2.5 ${on ? "bg-brand" : "bg-surface-2"}`}
            style={on ? sticker(colors, "sm") : undefined}
          >
            <Text className="text-xl">{emoji}</Text>
            <Text className={`text-xs font-black ${on ? "text-on-brand" : "text-ink"}`}>
              {t(`ghost.mode.${key}`)}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

/** Ghost mode for everyone, with a timer. */
export function GhostPanel({ now }: { now: number }) {
  const { t, i18n } = useTranslation("map");
  const { data: settings } = useGhostSettings();
  const setGhost = useSetGhost();
  const [timer, setTimer] = useState(0);
  const everyone = ghostRuleFor(
    settings ? ({ ...settings, overrides: {} } as geo.GetGhostSettingsResponse) : undefined,
    "",
    now,
  );
  const until = toMillis(settings?.everyone?.until);
  return (
    <View className="gap-3" testID="ghost-panel">
      <View>
        <Text className="text-lg font-black text-ink">👻 {t("ghost.title")}</Text>
        <Text className="text-sm text-ink-soft">{t("ghost.subtitle")}</Text>
      </View>
      <GhostPick
        current={everyone}
        label={t("ghost.everyone")}
        onPick={(mode) => setGhost.mutate({ userIds: [], mode, durationSeconds: timer })}
      />
      <Text className="rounded-control bg-surface-2 px-3 py-2 text-xs font-bold text-ink-soft">
        {t(`ghost.explain.${MODE_KEY[everyone] ?? "precise"}`)}
        {everyone !== geo.GhostMode.PRECISE && until > now
          ? `\n${t("ghost.backIn", { time: compactDuration(i18n.language, until - now) })}`
          : ""}
      </Text>
      <Text className="text-xs font-black text-ink-soft">{t("ghost.timer")}</Text>
      <View accessibilityRole="radiogroup" className="flex-row flex-wrap gap-1.5">
        {TIMERS.map((option) => (
          <Pressable
            key={option.key}
            accessibilityRole="radio"
            accessibilityState={{ checked: timer === option.seconds }}
            onPress={() => setTimer(option.seconds)}
            className={`rounded-full px-3.5 py-2 ${timer === option.seconds ? "bg-brand" : "bg-surface-2"}`}
          >
            <Text
              className={`text-xs font-black ${timer === option.seconds ? "text-on-brand" : "text-ink"}`}
            >
              {t(`ghost.timers.${option.key}`)}
            </Text>
          </Pressable>
        ))}
      </View>
      <Text className="text-[11px] font-bold text-ink-faint">{t("ghost.timerHint")}</Text>
    </View>
  );
}

/** The chip on the map while ghost mode hides me from anyone. */
export function ghostChipText(
  settings: geo.GetGhostSettingsResponse | undefined,
  now: number,
  t: TFunction<"map">,
  language: string,
): string | null {
  const summary = ghostSummary(settings, now);
  if (!summary) return null;
  const mode = MODE_KEY[summary.mode] ?? "blurred";
  const scope =
    summary.scope === "everyone"
      ? t("ghost.scopeEveryone")
      : t("ghost.scopeSome", { count: summary.hiddenCount });
  const left =
    summary.until && summary.until > now
      ? ` · ${t("ghost.timeLeft", { time: compactDuration(language, summary.until - now) })}`
      : "";
  return `${t(`ghost.on.${mode}`)} · ${scope}${left}`;
}
