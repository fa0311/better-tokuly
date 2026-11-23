import { useEffect, useState } from "react";
import { create } from "zustand";

/**
 * Slow mode as this browser knows it: when the person may post in a channel
 * again. The server enforces the rule; this only lets the composer say
 * "wait 24 s" instead of failing.
 */
const useSlowMode = create<Record<string, number>>(() => ({}));

/** Call after a post went through (or was refused with SLOW_MODE) in a channel with slow mode. */
export function noteSlowPost(channelId: string, seconds: number, nowMs = Date.now()): void {
  if (seconds > 0) useSlowMode.setState({ [channelId]: nowMs + seconds * 1000 });
}

/** Whole seconds until the person may post in the channel again; 0 when they may. */
export function secondsLeft(untilMs: number | undefined, nowMs: number): number {
  return untilMs && untilMs > nowMs ? Math.ceil((untilMs - nowMs) / 1000) : 0;
}

/** `secondsLeft` for a channel, counting down while it is above zero. */
export function useSlowWait(channelId: string | undefined): number {
  const until = useSlowMode((state) => (channelId ? state[channelId] : undefined));
  const [now, setNow] = useState(() => Date.now());
  const waiting = secondsLeft(until, now) > 0;
  useEffect(() => {
    if (!until) return;
    setNow(Date.now());
    if (until <= Date.now()) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [until]);
  return waiting ? secondsLeft(until, now) : 0;
}
