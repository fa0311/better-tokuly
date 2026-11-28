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
 * What the circle field does with the chosen `value` given the caller's
 * circles (`undefined` while they load): the circle to pick by itself (only
 * for an empty choice), what the select shows, and whether the chosen
 * circle is gone.
 */
export function circleChoice(
  value: string,
  circleIds: string[] | undefined,
): { autoPick?: string; selected: string; missing: boolean } {
  if (!circleIds) return { selected: value, missing: false };
  const known = circleIds.includes(value);
  return {
    autoPick: value === "" ? circleIds[0] : undefined,
    selected: known ? value : "",
    missing: value !== "" && !known,
  };
}

/**
 * Picks one of the caller's circles for a CIRCLE audience. Only a choice
 * that is still empty gets the first circle; a circle that is gone (deleted
 * since it was chosen) is never swapped for another one behind the
 * creator's back: the field says so and waits for a new choice.
 */
export function CircleField({
  value,
  onChange,
  label,
  emptyText,
  missingText,
}: {
  value: string;
  onChange(circleId: string): void;
  label: string;
  emptyText: string;
  /** Shown when `value` names a circle the caller no longer has. */
  missingText: string;
}) {
  const circles = useOwnCircles();
  const list = circles.data ?? [];
  const choice = circleChoice(
    value,
    circles.isSuccess ? list.map((circle) => circle.id) : undefined,
  );
  const autoPick = choice.autoPick;
  useEffect(() => {
    if (autoPick) onChange(autoPick);
  }, [autoPick, onChange]);
  if (circles.isSuccess && list.length === 0) {
    return (
      <p role="alert" className="text-sm font-normal text-danger">
        {emptyText}
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-1">
      <SelectField
        label={label}
        value={choice.selected}
        onChange={onChange}
        options={list.map((circle) => ({
          value: circle.id,
          label: `${circle.emoji ? `${circle.emoji} ` : ""}${circle.name}`,
        }))}
      />
      {choice.missing && (
        <p role="alert" className="text-sm font-normal text-danger" data-testid="circle-missing">
          {missingText}
        </p>
      )}
    </div>
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
