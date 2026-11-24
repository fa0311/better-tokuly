import { useQuery } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { Lock } from "lucide-react";
import { useEffect } from "react";
import { SelectField } from "#/components/ui/select";
import { followClient, followKeys } from "#/features/friends/follow-data";
import { api } from "#/lib/api";

/**
 * What the studios of videos, shorts and live streams share about audiences
 * (the social graph's: followers, close friends, circles) and locked accounts.
 * The strings come from the caller, so each feature speaks in its own words.
 */

/** The caller's custom circles (close friends has a choice of its own). */
export function useOwnCircles(enabled = true) {
  return useQuery({
    queryKey: ["social", "circles", "custom"],
    enabled,
    staleTime: 60_000,
    queryFn: async () =>
      (await api().service(social.CircleService).listCircles({})).circles.filter(
        (circle) => circle.kind === social.CircleKind.CUSTOM,
      ),
  });
}

/** Whether the caller's account is locked (鍵アカウント). */
export function useAccountLocked(): boolean {
  const settings = useQuery({
    queryKey: followKeys.settings,
    staleTime: 60_000,
    queryFn: () => followClient().getFollowSettings({}),
  });
  return settings.data?.settings?.locked ?? false;
}

/**
 * Picks one of the caller's circles for a CIRCLE audience. Picks the first
 * circle when none is chosen yet, so the form never sends a circle audience
 * without its circle.
 */
export function CircleField({
  value,
  onChange,
  label,
  emptyText,
}: {
  value: string;
  onChange(circleId: string): void;
  label: string;
  emptyText: string;
}) {
  const circles = useOwnCircles();
  const list = circles.data ?? [];
  const first = list[0]?.id;
  const known = list.some((circle) => circle.id === value);
  useEffect(() => {
    if (!known && first) onChange(first);
  }, [known, first, onChange]);
  if (circles.isSuccess && list.length === 0) {
    return (
      <p role="alert" className="text-sm font-medium text-danger">
        {emptyText}
      </p>
    );
  }
  return (
    <SelectField
      label={label}
      value={known ? value : (first ?? "")}
      onChange={onChange}
      options={list.map((circle) => ({
        value: circle.id,
        label: `${circle.emoji ? `${circle.emoji} ` : ""}${circle.name}`,
      }))}
    />
  );
}

/** A note shown next to the audience choice while the account is locked. */
export function LockedNote({ text }: { text: string }) {
  const locked = useAccountLocked();
  if (!locked) return null;
  return (
    <p className="flex items-start gap-2 text-sm text-ink-soft" data-testid="locked-note">
      <Lock className="mt-0.5 size-4 shrink-0" />
      <span>{text}</span>
    </p>
  );
}
