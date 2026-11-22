import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type common, video } from "@tokuly/api";
import { Bell, BellOff, BellRing, ChevronDown, UserMinus } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button, type ButtonProps } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { errorMessage } from "#/lib/errors";
import { channelClient, videoKeys } from "./api";

const { NotificationLevel } = video;

export interface SubscribeButtonProps {
  channel: Pick<common.ChannelSummary, "id" | "name">;
  /** Whether the viewer subscribes now, as the server last said. */
  subscribed: boolean;
  /** Their notification level; only meaningful while subscribed. */
  notificationLevel?: video.NotificationLevel;
  /** The video being watched, so the creator learns what won the subscriber. */
  sourceVideoId?: string;
  size?: ButtonProps["size"];
  className?: string;
}

/**
 * Subscribes to a channel. Once subscribed it becomes a menu for the
 * notification level (all, personalized, none) and for unsubscribing. Not for
 * the viewer's own channels: the server refuses those.
 */
export function SubscribeButton({ channel, subscribed, notificationLevel, sourceVideoId, size = "md", className }: SubscribeButtonProps) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  // What the last tap made of it, until the server's answer has spread through the cache.
  const [local, setLocal] = useState<{ subscribed: boolean; level: video.NotificationLevel; basis: string } | null>(null);
  // Once the server's state has moved on from what the tap was based on, it speaks again.
  const basis = `${subscribed}:${notificationLevel ?? 0}`;
  const pending = local?.basis === basis ? local : null;
  const isSubscribed = pending?.subscribed ?? subscribed;
  const level = pending?.level ?? (notificationLevel || NotificationLevel.PERSONALIZED);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: videoKeys.channels });
    void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
    void queryClient.invalidateQueries({ queryKey: videoKeys.subscriptionsFeed });
  };
  const onError = (error: unknown) => {
    setLocal(null);
    toast.error(errorMessage(error));
  };

  const subscribe = useMutation({
    mutationFn: (next: video.NotificationLevel) => channelClient().subscribe({ channelId: channel.id, notificationLevel: next, sourceVideoId: sourceVideoId ?? "" }),
    onMutate: (next) => {
      const first = !isSubscribed;
      setLocal({ subscribed: true, level: next || NotificationLevel.PERSONALIZED, basis });
      return { first };
    },
    onSuccess: (_response, _next, context) => {
      if (context?.first) toast.success(t("subscribe.done", { name: channel.name }));
      refresh();
    },
    onError,
  });
  const unsubscribe = useMutation({
    mutationFn: () => channelClient().unsubscribe({ channelId: channel.id }),
    onMutate: () => setLocal({ subscribed: false, level: NotificationLevel.PERSONALIZED, basis }),
    onSuccess: () => {
      toast.message(t("subscribe.undone", { name: channel.name }));
      refresh();
    },
    onError,
  });

  if (!isSubscribed) {
    return (
      <Button size={size} className={className} loading={subscribe.isPending} onClick={() => subscribe.mutate(NotificationLevel.UNSPECIFIED)}>
        {t("subscribe.action")}
      </Button>
    );
  }

  const LevelIcon = level === NotificationLevel.ALL ? BellRing : level === NotificationLevel.NONE ? BellOff : Bell;
  const levelAction = (value: video.NotificationLevel, key: string, icon: ReactNode) => ({
    label: `${level === value ? "✓ " : ""}${t(`subscribe.level.${key}`)}`,
    icon,
    onSelect: () => subscribe.mutate(value),
  });
  return (
    <Menu
      trigger={
        <Button variant="secondary" size={size} className={className} aria-label={t("subscribe.manage", { name: channel.name })}>
          <LevelIcon className="size-4" />
          {t("subscribe.subscribed")}
          <ChevronDown className="size-4" />
        </Button>
      }
      actions={[
        levelAction(NotificationLevel.ALL, "all", <BellRing />),
        levelAction(NotificationLevel.PERSONALIZED, "personalized", <Bell />),
        levelAction(NotificationLevel.NONE, "none", <BellOff />),
        { label: t("subscribe.cancel"), icon: <UserMinus />, danger: true, onSelect: () => unsubscribe.mutate() },
      ]}
    />
  );
}
