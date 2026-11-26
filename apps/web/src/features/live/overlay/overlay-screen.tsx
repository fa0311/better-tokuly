import { errorReason, live } from "@tokuly/api";
import { pushesDuringRead } from "@tokuly/core/query";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { realtime } from "#/lib/realtime";
import { giftClient, overlayClient } from "../api";
import { addChatMessages, mergePoll, removeChatMessage, removeChatUser } from "../model";
import { AlertWidget, ChatBoxWidget, GoalWidget, PollWidget, WIDGET_BASE_CSS } from "./widgets";

const { OverlayKind, AlertKind } = live;

interface WidgetState {
  overlay: live.Overlay;
  css: string;
  topic: string;
  streamId: string;
  messages: live.ChatMessage[];
  goal: live.GoalProgress | undefined;
  poll: live.Poll | undefined;
}

function toState(response: live.GetOverlayResponse): WidgetState | null {
  if (!response.overlay) return null;
  return {
    overlay: response.overlay,
    css: response.css,
    topic: response.realtimeTopic,
    streamId: response.stream?.id ?? "",
    messages: response.recentMessages,
    goal: response.goal,
    poll: response.poll,
  };
}

function alertSettings(overlay: live.Overlay): live.AlertBoxSettings | undefined {
  return overlay.settings?.widget.case === "alertBox" ? overlay.settings.widget.value : undefined;
}

/** Whether an alert box set up like this announces the alert (samples always play). */
function wanted(alert: live.Alert, settings: live.AlertBoxSettings | undefined): boolean {
  if (alert.test || !settings) return true;
  if (alert.kind === AlertKind.GIFT)
    return !settings.hideGifts && alert.coins >= settings.minGiftCoins;
  if (alert.kind === AlertKind.SUBSCRIBER) return !settings.hideSubscribers;
  if (alert.kind === AlertKind.RAID) return !settings.hideRaids;
  return true;
}

/**
 * `/overlay/<token>`: one OBS widget, and nothing but the widget — a
 * transparent page for a browser source. The token in the address is the
 * widget's secret: it goes into the one API call that opens the widget and
 * nowhere else (the realtime topic is a separate key the API hands out).
 */
export function OverlayScreen({ token }: { token: string }) {
  const { t } = useTranslation("live");
  const [state, setState] = useState<WidgetState | null>(null);
  const [gone, setGone] = useState(false);
  const [tiers, setTiers] = useState<live.GiftTier[]>([]);
  const [alert, setAlert] = useState<live.Alert | null>(null);
  const queue = useRef<live.Alert[]>([]);
  const showing = useRef(false);
  /** Changing it reads the widget again (its configuration or its stream changed). */
  const [version, setVersion] = useState(0);
  /** What pushes changed while the widget is being read: the answer may predate it. */
  const [duringRead] = useState(() => pushesDuringRead<WidgetState>());

  useEffect(() => {
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const load = async (delay: number) => {
      const read = duringRead.begin();
      try {
        const answer = toState(await overlayClient().getOverlay({ token }));
        const next = answer && read.finish(answer);
        read.cancel();
        if (cancelled) return;
        setState(next);
        setGone(next === null);
      } catch (error) {
        read.cancel();
        if (cancelled) return;
        if (errorReason(error) === "OVERLAY_NOT_FOUND") {
          setState(null);
          setGone(true);
          return;
        }
        // A busy or restarting server: the widget keeps what it shows and asks again.
        retry = setTimeout(() => void load(Math.min(delay * 2, 30_000)), delay);
      }
    };
    void load(2000);
    return () => {
      cancelled = true;
      clearTimeout(retry);
    };
  }, [token, version, duringRead]);

  useEffect(() => {
    giftClient()
      .getGiftTiers({})
      .then((response) => setTiers(response.tiers))
      .catch(() => {});
  }, []);

  const topic = state?.topic ?? "";
  const overlayId = state?.overlay.id ?? "";
  const duration = (state && alertSettings(state.overlay)?.durationSeconds) || 6;
  const latest = useRef({ state, duration });
  useEffect(() => {
    latest.current = { state, duration };
  });

  useEffect(() => {
    if (!topic) return;
    const client = realtime();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const next = () => {
      const upcoming = queue.current.shift();
      if (!upcoming) {
        showing.current = false;
        setAlert(null);
        return;
      }
      showing.current = true;
      setAlert(upcoming);
      // A short gap between two alerts lets the entrance animation play again.
      timer = setTimeout(() => {
        setAlert(null);
        timer = setTimeout(next, 400);
      }, latest.current.duration * 1000);
    };
    const offFrame = client.onFrame((frame) => {
      if (frame.body.case !== "live" || frame.topic !== topic) return;
      const event = frame.body.value.event;
      const patch = (update: (current: WidgetState) => WidgetState) => {
        duringRead.record(update);
        setState((current) => (current ? update(current) : current));
      };
      switch (event.case) {
        case "chatMessage":
          patch((current) => ({
            ...current,
            messages: addChatMessages(current.messages, [event.value], 60),
          }));
          break;
        case "chatMessageDeleted":
          patch((current) => ({
            ...current,
            messages: removeChatMessage(current.messages, event.value.messageId),
          }));
          break;
        case "chatUserCleared":
          patch((current) => ({
            ...current,
            messages: removeChatUser(current.messages, event.value.userId),
          }));
          break;
        case "alert": {
          const current = latest.current.state;
          if (current && !wanted(event.value, alertSettings(current.overlay))) break;
          queue.current.push(event.value);
          if (!showing.current) next();
          break;
        }
        case "goalProgress":
          patch((current) => ({ ...current, goal: event.value }));
          break;
        case "pollUpdated":
          patch((current) => ({ ...current, poll: mergePoll(current.poll, event.value) }));
          break;
        case "streamUpdated":
          // Another broadcast began (or this one ended): its chat and poll are read afresh.
          if (event.value.id !== latest.current.state?.streamId) setVersion((value) => value + 1);
          break;
        case "overlayChanged":
          if (event.value.revoked) {
            setState(null);
            setGone(true);
          } else setVersion((value) => value + 1);
          break;
        default:
          break;
      }
    });
    const { release } = client.subscribe(topic, {
      // The server has the subscription in place (the first time, or back after a gap):
      // whatever was pushed before that, including while the widget was first being read,
      // is read in one go.
      onSubscribed: () => setVersion((value) => value + 1),
      // The widget was deleted or its token renewed while this page was not listening.
      onRevoked: () => {
        setState(null);
        setGone(true);
      },
    });
    client.start();
    return () => {
      clearTimeout(timer);
      queue.current = [];
      showing.current = false;
      offFrame();
      release();
    };
  }, [topic, duringRead]);

  if (!state)
    return gone ? (
      <p
        style={{
          margin: 16,
          font: "600 14px system-ui, sans-serif",
          color: "#ffffff",
          textShadow: "0 1px 3px #000",
        }}
        data-testid="overlay-gone"
      >
        {t("overlay.gone")}
      </p>
    ) : null;

  const { overlay } = state;
  const settings = overlay.settings?.widget;
  return (
    <div data-testid="overlay-root" data-kind={overlay.kind} data-overlay={overlayId}>
      {/* The preset and the streamer's CSS: validated by the API to load nothing. */}
      <style>{`${WIDGET_BASE_CSS}\n${state.css}`}</style>
      {overlay.kind === OverlayKind.CHAT_BOX && (
        <ChatBoxWidget
          messages={state.messages}
          settings={settings?.case === "chatBox" ? settings.value : undefined}
          tiers={tiers}
        />
      )}
      {overlay.kind === OverlayKind.ALERT_BOX && (
        <AlertWidget alert={alert} settings={alertSettings(overlay)} />
      )}
      {overlay.kind === OverlayKind.GOAL && (
        <GoalWidget
          goal={state.goal}
          settings={settings?.case === "goal" ? settings.value : undefined}
        />
      )}
      {overlay.kind === OverlayKind.POLL && (
        <PollWidget
          poll={state.poll}
          settings={settings?.case === "poll" ? settings.value : undefined}
        />
      )}
    </div>
  );
}
