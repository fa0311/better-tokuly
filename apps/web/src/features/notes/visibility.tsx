import { useQuery } from "@tanstack/react-query";
import { common, social } from "@tokuly/api";
import {
  AUDIENCE_OPTIONS,
  type Audience,
  type AudienceKey,
  type AudienceOption,
  audienceOption,
  notesKeys,
} from "@tokuly/core/notes";
import {
  AtSign,
  Check,
  CircleDot,
  Globe,
  Lock,
  Star,
  UserRoundCheck,
  UsersRound,
} from "lucide-react";
import type { ComponentType } from "react";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";

/** The icon of each audience. */
export const AUDIENCE_ICONS: Record<AudienceKey, ComponentType<{ className?: string }>> = {
  public: Globe,
  followers: UserRoundCheck,
  friends: UsersRound,
  closeFriends: Star,
  circle: CircleDot,
  direct: AtSign,
  private: Lock,
};

/** The custom circles of the viewer (close friends has its own choice). */
export function useCustomCircles(enabled = true) {
  return useQuery({
    queryKey: notesKeys.circles,
    enabled,
    staleTime: 60_000,
    queryFn: async () =>
      (await api().service(social.CircleService).listCircles({})).circles.filter(
        (circle) => circle.kind === social.CircleKind.CUSTOM,
      ),
  });
}

/** The viewer's friends, to address a direct note to. */
export function useDirectCandidates(enabled: boolean) {
  return useQuery({
    queryKey: notesKeys.directCandidates,
    enabled,
    staleTime: 60_000,
    queryFn: async () =>
      (
        await api()
          .service(social.FriendService)
          .listFriends({ page: { limit: 100 } })
      ).friends
        .map((friend) => friend.user)
        .filter((user): user is common.UserSummary => Boolean(user)),
  });
}

/** A small marker on notes that are not shared with everyone: an icon with the audience as its label. */
export function VisibilityMark({
  visibility,
  direct,
  className,
}: {
  visibility: common.Visibility;
  /** The note names people (a direct note). */
  direct?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("notes");
  if (visibility === common.Visibility.PUBLIC || visibility === common.Visibility.UNSPECIFIED)
    return null;
  const shown = audienceOption({ visibility, directUserIds: direct ? ["?"] : [] });
  const Icon = AUDIENCE_ICONS[shown.key];
  return (
    <span
      role="img"
      aria-label={t(`visibility.${shown.key}.label`)}
      title={t(`visibility.${shown.key}.label`)}
      className={cn(
        "inline-flex shrink-0",
        shown.key === "closeFriends" ? "text-success" : "text-ink-faint",
        className,
      )}
    >
      <Icon className={cn("size-3.5", shown.key === "closeFriends" && "fill-current")} />
    </span>
  );
}

/**
 * The audiences as a list of choices, each with a line saying who that is:
 * the fixed ones, then each of the viewer's custom circles, then named
 * people (with the friends to pick from) and "only me".
 */
export function VisibilityChoices({
  value,
  onChange,
  disabled,
  minRank = 0,
}: {
  value: Audience;
  onChange(value: Audience): void;
  disabled?: boolean;
  /** A reply cannot be shown more widely than the note it answers. */
  minRank?: number;
}) {
  const { t } = useTranslation("notes");
  const circles = useCustomCircles();
  const current = audienceOption(value);
  const direct = current.key === "direct";
  const friends = useDirectCandidates(direct);
  const choices: {
    id: string;
    option: AudienceOption;
    label: string;
    hint: string;
    audience: Audience;
  }[] = [];
  for (const candidate of AUDIENCE_OPTIONS) {
    if (candidate.key === "circle") {
      for (const circle of circles.data ?? [])
        choices.push({
          id: `circle:${circle.id}`,
          option: candidate,
          label: circle.emoji ? `${circle.emoji} ${circle.name}` : circle.name,
          hint: t("visibility.circle.hint"),
          audience: { visibility: common.Visibility.CIRCLE, circleId: circle.id },
        });
      continue;
    }
    choices.push({
      id: candidate.key,
      option: candidate,
      label: t(`visibility.${candidate.key}.label`),
      hint: t(`visibility.${candidate.key}.hint`),
      audience:
        candidate.key === "direct"
          ? {
              visibility: common.Visibility.PRIVATE,
              directUserIds: value.directUserIds ?? [],
              direct: true,
            }
          : { visibility: candidate.value },
    });
  }
  const selectedId = current.key === "circle" ? `circle:${value.circleId}` : current.key;
  const people = new Set(value.directUserIds ?? []);
  return (
    <div role="radiogroup" aria-label={t("visibility.title")} className="flex flex-col gap-1.5">
      {choices.map((choice) => {
        const Icon = AUDIENCE_ICONS[choice.option.key];
        const selected = choice.id === selectedId;
        return (
          <div key={choice.id} className="flex flex-col gap-1.5">
            {/* biome-ignore lint/a11y/useSemanticElements: a radio that carries an icon and a description, not a bare radio input. */}
            <button
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={disabled || choice.option.rank < minRank}
              onClick={() => onChange(choice.audience)}
              className={cn(
                "flex items-center gap-3 rounded-2xl border-2 px-3 py-2.5 text-left transition-colors disabled:opacity-50",
                selected ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint",
              )}
            >
              <span
                className={cn(
                  "grid size-9 shrink-0 place-items-center rounded-xl border-2 border-ink",
                  selected ? "bg-lime text-[#17122b]" : "bg-surface",
                )}
              >
                <Icon className="size-[18px]" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block font-black">{choice.label}</span>
                <span className="block text-sm text-ink-soft">{choice.hint}</span>
              </span>
              {selected && <Check className="size-5 shrink-0 text-brand" strokeWidth={3} />}
            </button>
            {selected && choice.id === "direct" && (
              <fieldset
                aria-label={t("visibility.direct.pick")}
                className="ml-12 flex max-h-48 flex-col gap-1 overflow-y-auto"
              >
                {(friends.data ?? []).length === 0 && (
                  <p className="text-sm text-ink-soft">{t("visibility.direct.noFriends")}</p>
                )}
                {(friends.data ?? []).map((friend) => (
                  <label key={friend.id} className="flex items-center gap-2 text-sm font-bold">
                    <input
                      type="checkbox"
                      checked={people.has(friend.id)}
                      disabled={disabled}
                      onChange={(event) => {
                        const next = new Set(people);
                        if (event.target.checked) next.add(friend.id);
                        else next.delete(friend.id);
                        onChange({
                          visibility: common.Visibility.PRIVATE,
                          directUserIds: [...next],
                          direct: true,
                        });
                      }}
                    />
                    {friend.displayName || friend.handle}
                    <span className="text-ink-faint">@{friend.handle}</span>
                  </label>
                ))}
              </fieldset>
            )}
          </div>
        );
      })}
    </div>
  );
}
