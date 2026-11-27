import { useQueryClient } from "@tanstack/react-query";
import { spaces, toMillis } from "@tokuly/api";
import { type Exit, minutesOf, nameOf, rememberSpace } from "@tokuly/core/spaces";
import { router } from "expo-router";
import { Bell, BellRing } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, ScrollView, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { sticker, useTheme } from "#/lib/theme";
import { spaceClient } from "./data";
import { formatWhen } from "./parts";
import { acknowledgeExit, enterHangout, enterSpace, startSpace } from "./session";

const Role = spaces.SpaceRole;

/** A room that has not started: when, who hosts, the reminder; the host starts it. */
export function UpcomingRoom({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const reminded = Boolean(space.viewer?.reminderSet);
  const host = space.viewer?.role === Role.HOST;

  const remind = async () => {
    setBusy(true);
    try {
      const response = await spaceClient().setReminder({ spaceId: space.id, remind: !reminded });
      rememberSpace(queryClient, response.space);
      Alert.alert(t(reminded ? "toast.reminderOff" : "toast.reminderOn"));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "spaces" }));
    }
    setBusy(false);
  };
  const start = async () => {
    setBusy(true);
    await startSpace(queryClient, space.id);
    setBusy(false);
  };

  return (
    <ScrollView testID="space-upcoming" contentContainerClassName="gap-5 px-4 pt-4 pb-12">
      <View className="gap-3 rounded-card bg-surface p-5" style={sticker(colors)}>
        <Text className="text-sm font-black text-brand">
          {formatWhen(toMillis(space.scheduledAt))}
        </Text>
        <Text className="text-2xl leading-8 font-black text-ink">{space.title}</Text>
        {space.tags.length > 0 && (
          <Text className="text-sm font-bold text-brand">
            {space.tags.map((tag) => `#${tag}`).join("  ")}
          </Text>
        )}
        <View className="flex-row items-center gap-3">
          <Avatar user={space.host} outlined />
          <View>
            <Text className="text-base font-black text-ink">{nameOf(space.host)}</Text>
            <Text className="text-xs text-ink-soft">{t("role.host")}</Text>
          </View>
        </View>
        <Text className="text-sm font-bold text-ink-soft">
          {space.reminderCount > 0
            ? t("scheduled.reminders", { count: space.reminderCount })
            : t("scheduled.noReminders")}
        </Text>
        {space.viewer?.invited && (
          <Text className="text-sm font-black text-pink">💌 {t("scheduled.invited")}</Text>
        )}
      </View>
      {host ? (
        <Button testID="space-start" size="lg" block loading={busy} onPress={() => void start()}>
          {t("mine.start")}
        </Button>
      ) : (
        <Button
          testID="space-remind"
          size="lg"
          block
          variant={reminded ? "secondary" : "primary"}
          loading={busy}
          icon={
            reminded ? (
              <BellRing size={18} color={colors.ink} />
            ) : (
              <Bell size={18} color={colors["on-brand"]} />
            )
          }
          onPress={() => void remind()}
        >
          {t(reminded ? "reminder.on" : "reminder.off")}
        </Button>
      )}
    </ScrollView>
  );
}

/** A room that is over or was called off: why, and how it went. */
export function EndedRoom({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const { colors } = useTheme();
  const cancelled = space.state === spaces.SpaceState.CANCELLED;
  const Reason = spaces.SpaceEndReason;
  const reason =
    space.endReason === Reason.MODERATION
      ? "moderation"
      : space.endReason === Reason.HOST_ENDED
        ? "hostEnded"
        : space.endReason === Reason.NEVER_STARTED
          ? "neverStarted"
          : space.endReason === Reason.EMPTY
            ? "empty"
            : "hostLeft";
  const minutes = minutesOf(space);
  return (
    <ScrollView testID="space-ended" contentContainerClassName="gap-4 px-4 pt-4 pb-12">
      <View className="gap-3 rounded-card bg-surface p-5" style={sticker(colors)}>
        <Text className="text-xs font-black text-ink-soft">
          {t(cancelled ? "ended.cancelledTitle" : "ended.title")}
        </Text>
        <Text className="text-2xl leading-8 font-black text-ink">{space.title}</Text>
        <Text className="text-sm text-ink-soft">{t(`ended.reason.${reason}`)}</Text>
        {!cancelled && (
          <View className="flex-row gap-2 pt-2">
            {[
              [t("ended.minutes"), t("ended.minutesValue", { count: minutes })],
              [t("ended.peak"), space.peakListeners.toLocaleString()],
              [t("ended.total"), space.totalListeners.toLocaleString()],
            ].map(([label, value]) => (
              <View key={label} className="flex-1 items-center rounded-2xl bg-surface-2 py-3">
                <Text className="text-lg font-black text-ink">{value}</Text>
                <Text className="text-[11px] font-bold text-ink-soft">{label}</Text>
              </View>
            ))}
          </View>
        )}
      </View>
    </ScrollView>
  );
}

/** Why the person is outside although they did not leave, and the way back in when there is one. */
export function ExitNotice({ exit }: { exit: Exit }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const again = exit.reason === "elsewhere" || exit.reason === "lost";
  const emoji = {
    removed: "🚪",
    banned: "⛔",
    ended: "👋",
    closed: "🫥",
    elsewhere: "📱",
    lost: "📶",
  }[exit.reason];
  const enter = () => {
    acknowledgeExit();
    if (exit.kind === "space") void enterSpace(queryClient, { id: exit.id, title: "" });
    else void enterHangout(queryClient, { id: exit.id, name: "", emoji: "" });
  };
  return (
    <View testID="space-exit" className="flex-1">
      <EmptyState
        emoji={emoji}
        title={t(`exit.${exit.reason}.title`)}
        body={t(`exit.${exit.reason}.body`)}
        action={
          <View className="items-center gap-2">
            {again && (
              <Button testID="exit-again" onPress={enter}>
                {t(exit.reason === "lost" ? "exit.reconnect" : "exit.enterAgain")}
              </Button>
            )}
            <Button
              variant="ghost"
              onPress={() => {
                acknowledgeExit();
                router.back();
              }}
            >
              {t("exit.toList")}
            </Button>
          </View>
        }
      />
    </View>
  );
}
