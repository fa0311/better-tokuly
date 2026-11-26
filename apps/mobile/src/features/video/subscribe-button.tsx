import { type common, video } from "@tokuly/api";
import { Bell } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ActionSheet } from "#/components/action-sheet";
import { Button } from "#/components/button";
import { useTheme } from "#/lib/theme";
import { useSubscribe } from "./data";

const { NotificationLevel } = video;

const LEVELS = [
  { level: NotificationLevel.ALL, key: "all" },
  { level: NotificationLevel.PERSONALIZED, key: "personalized" },
  { level: NotificationLevel.NONE, key: "none" },
] as const;

/**
 * Subscribes to a channel. Once subscribed, a tap opens the notification
 * level (all, personalized, none) and the way out. The tap shows at once;
 * the server's state takes over once it has moved on from what the tap was
 * based on. Not for the viewer's own channels: the server refuses those.
 */
export function SubscribeButton({
  channel,
  subscribed,
  notificationLevel,
  sourceVideoId,
}: {
  channel: Pick<common.ChannelSummary, "id" | "name">;
  subscribed: boolean;
  notificationLevel?: video.NotificationLevel;
  /** The video being watched, so the creator learns what won the subscriber. */
  sourceVideoId?: string;
}) {
  const { t } = useTranslation("video");
  const { colors } = useTheme();
  const change = useSubscribe();
  const [menu, setMenu] = useState(false);
  const basis = `${subscribed}:${notificationLevel ?? 0}`;
  const [local, setLocal] = useState<{ subscribed: boolean; basis: string } | null>(null);
  const isSubscribed = local?.basis === basis ? local.subscribed : subscribed;

  const set = async (next: boolean, level?: video.NotificationLevel) => {
    setLocal({ subscribed: next, basis });
    if (!(await change(channel.id, next, { level, sourceVideoId }))) setLocal(null);
  };

  if (!isSubscribed)
    return (
      <Button testID="video-subscribe" size="sm" onPress={() => void set(true)}>
        {t("subscribe.action")}
      </Button>
    );
  return (
    <>
      <Button
        testID="video-subscribed"
        size="sm"
        variant="secondary"
        icon={<Bell color={colors.ink} size={16} />}
        onPress={() => setMenu(true)}
      >
        {t("subscribe.subscribed")}
      </Button>
      <ActionSheet
        visible={menu}
        title={t("subscribe.manage", { name: channel.name })}
        onClose={() => setMenu(false)}
        actions={[
          ...LEVELS.map(({ level, key }) => ({
            key: `level-${key}`,
            label: `${(notificationLevel || NotificationLevel.PERSONALIZED) === level ? "✓ " : ""}${t(`subscribe.level.${key}`)}`,
            onPress: () => void set(true, level),
          })),
          {
            key: "unsubscribe",
            label: t("subscribe.cancel"),
            destructive: true,
            onPress: () => void set(false),
          },
        ]}
      />
    </>
  );
}
