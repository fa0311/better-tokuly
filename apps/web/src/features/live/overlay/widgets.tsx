import { live } from "@tokuly/api";
import { pollShares, tierByLevel } from "@tokuly/core/live";
import { useTranslation } from "react-i18next";
import { EmojiText } from "#/features/emoji";
import { ROLE_KEYS } from "../labels";
import { fillAlertTemplate, tierStyle } from "../model";

/**
 * The widgets of OBS overlays, as plain markup. Their looks come entirely
 * from the widget's CSS (a preset followed by the streamer's own), which
 * styles these class names — the contract with the backend's presets:
 * `.chat-body`, `.chat-message`, `.chat-avatar`, `.chat-message-content`,
 * `.chat-message-name`, `.chat-message-text`, `.chat-badge`,
 * `.chat-gift-message`, `.chat-gift-amount`; `.alert`, `.alert-title`,
 * `.alert-message`; `.goal`, `.goal-title`, `.goal-bar`, `.goal-fill`,
 * `.goal-numbers`; `.poll`, `.poll-question`, `.poll-option`, `.poll-bar`,
 * `.poll-votes`. Nothing here uses the app's own styles.
 */

const { ChatMessageKind, AlertKind, GoalKind } = live;

export function ChatBoxWidget({
  messages,
  settings,
  tiers,
}: {
  /** Oldest first. */
  messages: live.ChatMessage[];
  settings: live.ChatBoxSettings | undefined;
  tiers: live.GiftTier[];
}) {
  const { t, i18n } = useTranslation("live");
  const shown = messages
    .filter((message) => {
      if (message.kind === ChatMessageKind.NOTICE) return false;
      if (message.kind === ChatMessageKind.STAMP) return !settings?.hideStamps && message.stamp;
      if (message.kind === ChatMessageKind.GIFT) return !settings?.hideGifts;
      return true;
    })
    .slice(-(settings?.maxMessages || 20));
  return (
    <div className="chat-body" data-testid="overlay-chat">
      {shown.map((message) => {
        const name = message.author?.channel?.name || message.author?.user?.displayName || "";
        const avatar = message.author?.channel?.icon?.url || message.author?.user?.avatar?.url;
        const roleKey = ROLE_KEYS[message.role];
        const fade = settings?.fadeSeconds
          ? { animation: `overlay-fade 600ms ease-in ${settings.fadeSeconds}s forwards` }
          : undefined;
        if (message.kind === ChatMessageKind.GIFT) {
          return (
            <div
              key={message.id}
              className="chat-message chat-gift-message"
              style={{ ...tierStyle(tierByLevel(tiers, message.gift?.tier ?? 0)), ...fade }}
              data-tier={message.gift?.tier}
            >
              <div
                className="chat-message-content"
                style={{ background: "transparent", boxShadow: "none" }}
              >
                <span className="chat-message-name" style={{ color: "inherit" }}>
                  {name}
                </span>
                <span className="chat-gift-amount">
                  {t("overlay.coins", {
                    coins: Number(message.gift?.coins ?? 0n).toLocaleString(i18n.language),
                  })}
                </span>
                {message.text && (
                  <div className="chat-message-text" style={{ color: "inherit" }}>
                    <EmojiText text={message.text} />
                  </div>
                )}
              </div>
            </div>
          );
        }
        return (
          <div key={message.id} className="chat-message" style={fade}>
            {!settings?.hideAvatars && avatar && (
              <img className="chat-avatar" src={avatar} alt="" />
            )}
            <div className="chat-message-content">
              <span className="chat-message-name">
                {name}
                {!settings?.hideBadges && roleKey && (
                  <span className="chat-badge">{t(`chatRole.${roleKey}`)}</span>
                )}
              </span>
              {message.kind === ChatMessageKind.STAMP && message.stamp ? (
                <img
                  src={message.stamp.imageUrl}
                  alt={message.stamp.name}
                  style={{ display: "block", width: 72, height: 72, objectFit: "contain" }}
                />
              ) : (
                <span className="chat-message-text">
                  <EmojiText text={message.text} />
                </span>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** The words of an alert: the streamer's template when they wrote one, the built-in wording otherwise. */
export function alertTitle(
  alert: live.Alert,
  settings: live.AlertBoxSettings | undefined,
  t: (key: string, values: Record<string, string>) => string,
  language: string,
): string {
  const values = {
    name: alert.name,
    coins: Number(alert.coins).toLocaleString(language),
    message: alert.message,
    count: Number(alert.viewerCount).toLocaleString(language),
  };
  const [template, key] =
    alert.kind === AlertKind.SUBSCRIBER
      ? [settings?.subscriberTemplate, "overlay.alert.subscriber"]
      : alert.kind === AlertKind.RAID
        ? [settings?.raidTemplate, "overlay.alert.raid"]
        : [settings?.giftTemplate, "overlay.alert.gift"];
  return template ? fillAlertTemplate(template, values) : t(key, values);
}

export function AlertWidget({
  alert,
  settings,
}: {
  /** The alert on screen; `null` shows nothing. */
  alert: live.Alert | null;
  settings: live.AlertBoxSettings | undefined;
}) {
  const { t, i18n } = useTranslation("live");
  if (!alert) return null;
  const title = alertTitle(alert, settings, t as never, i18n.language);
  // A template that already places the message does not get it twice.
  const custom =
    alert.kind === AlertKind.GIFT && Boolean(settings?.giftTemplate?.includes("{message}"));
  return (
    <div className="alert" data-testid="overlay-alert" data-kind={alert.kind}>
      <div className="alert-title">{title}</div>
      {alert.message && !custom && (
        <div className="alert-message">
          <EmojiText text={alert.message} />
        </div>
      )}
    </div>
  );
}

export function GoalWidget({
  goal,
  settings,
}: {
  goal: live.GoalProgress | undefined;
  settings: live.GoalSettings | undefined;
}) {
  const { t, i18n } = useTranslation("live");
  if (!goal) return null;
  const target = Number(goal.target);
  const current = Number(goal.current);
  const percent = target > 0 ? Math.min(100, (current / target) * 100) : 0;
  return (
    <div className="goal" data-testid="overlay-goal">
      <div className="goal-title">
        {goal.title ||
          t(goal.kind === GoalKind.SUBSCRIBERS ? "overlay.goal.subscribers" : "overlay.goal.coins")}
      </div>
      <div className="goal-bar">
        <div className="goal-fill" style={{ width: `${percent}%` }} />
        {!settings?.hideNumbers && (
          <div className="goal-numbers">
            {current.toLocaleString(i18n.language)} / {target.toLocaleString(i18n.language)}
          </div>
        )}
      </div>
    </div>
  );
}

export function PollWidget({
  poll,
  settings,
}: {
  poll: live.Poll | undefined;
  settings: live.PollWidgetSettings | undefined;
}) {
  const { i18n } = useTranslation("live");
  if (!poll || (settings?.hideWhenIdle && poll.endedAt)) return null;
  const shares = pollShares(poll);
  return (
    <div className="poll" data-testid="overlay-poll">
      <div className="poll-question">{poll.question}</div>
      {poll.options.map((option, index) => (
        <div key={index} className="poll-option">
          <div className="poll-bar" style={{ width: `${shares[index]}%` }} />
          <span style={{ position: "relative" }}>{option.text}</span>
          <span className="poll-votes" style={{ position: "relative" }}>
            {settings?.hideVoteCounts
              ? `${shares[index]}%`
              : Number(option.voteCount).toLocaleString(i18n.language)}
          </span>
        </div>
      ))}
    </div>
  );
}

/** Keyframes the widgets themselves need (a chat line fading out), independent of the preset. */
export const WIDGET_BASE_CSS = "@keyframes overlay-fade { to { opacity: 0; visibility: hidden; } }";
