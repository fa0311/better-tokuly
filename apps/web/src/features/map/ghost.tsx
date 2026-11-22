import { useMutation, useQueryClient } from "@tanstack/react-query";
import { geo, toMillis } from "@tokuly/api";
import { Ghost, Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactDuration } from "#/lib/format";
import {
  friendName,
  locationClient,
  mapKeys,
  useFriendsById,
  useGhostSettings,
  useNow,
} from "./data";
import { GHOST_MODES, GhostQuickPick } from "./friend-sheet";
import { ghostRuleFor, ghostSummary } from "./model";
import { MapSheet, SheetSection } from "./sheet";
import { useMapUi } from "./ui-store";

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

/**
 * Always on the map while ghost mode hides me from anyone: what is on, for
 * whom, and for how much longer. One tap opens the switch.
 */
export function GhostIndicator() {
  const { t } = useTranslation("map");
  const { data } = useGhostSettings();
  const now = useNow(30_000);
  const open = useMapUi((state) => state.open);
  const summary = ghostSummary(data, now);
  if (!summary) return null;
  const mode = MODE_KEY[summary.mode] ?? "blurred";
  return (
    <button
      type="button"
      data-testid="ghost-indicator"
      data-mode={mode}
      onClick={() => open({ kind: "ghost" })}
      className="sticker-sm pressable pointer-events-auto flex h-9 max-w-full items-center gap-1.5 rounded-full bg-ink px-3 text-xs font-black text-surface"
    >
      <Ghost className="size-4 shrink-0" />
      <span className="truncate">
        {t(`ghost.on.${mode}`)} ·{" "}
        {summary.scope === "everyone"
          ? t("ghost.scopeEveryone")
          : t("ghost.scopeSome", { count: summary.hiddenCount })}
        {summary.until && summary.until > now
          ? ` · ${t("ghost.timeLeft", { time: compactDuration(summary.until - now) })}`
          : ""}
      </span>
    </button>
  );
}

/** Ghost mode: how precisely friends see me — everyone, or one by one, with a timer. */
export function GhostPanel() {
  const { t } = useTranslation("map");
  const queryClient = useQueryClient();
  const { stack, back, close } = useMapUi();
  const { data: settings, isPending } = useGhostSettings();
  const friends = useFriendsById();
  const now = useNow(30_000);
  const [timer, setTimer] = useState(0);
  const [search, setSearch] = useState("");

  const setFor = useMutation({
    mutationFn: ({ userId, mode }: { userId: string; mode: geo.GhostMode }) =>
      locationClient().setGhostMode({ userIds: [userId], mode, durationSeconds: timer }),
    onSuccess: (response) =>
      response.settings && queryClient.setQueryData(mapKeys.ghost, response.settings),
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "map" }));
      void queryClient.invalidateQueries({ queryKey: mapKeys.ghost });
    },
  });

  const everyone = ghostRuleFor(
    settings ? ({ ...settings, overrides: {} } as geo.GetGhostSettingsResponse) : undefined,
    "",
    now,
  );
  const everyoneUntil = toMillis(settings?.everyone?.until);
  const needle = search.trim().toLowerCase();
  const rows = Object.values(friends)
    .filter((friend) => friend.user)
    .filter(
      (friend) =>
        !needle ||
        friendName(friend, friend.user).toLowerCase().includes(needle) ||
        friend.user?.handle.toLowerCase().includes(needle),
    )
    .sort((a, b) => friendName(a, a.user).localeCompare(friendName(b, b.user)));

  return (
    <MapSheet
      testId="ghost-panel"
      title={t("ghost.title")}
      subtitle={t("ghost.subtitle")}
      leading={<span className="text-2xl">👻</span>}
      onBack={stack.length > 1 ? back : undefined}
      onClose={close}
    >
      {isPending ? (
        <Skeleton className="h-24" />
      ) : (
        <>
          <SheetSection title={t("ghost.everyone")}>
            <GhostQuickPick userIds={[]} current={everyone} durationSeconds={timer} />
            <p
              className="mt-2 rounded-2xl bg-surface-2 px-3 py-2 text-xs leading-relaxed font-bold text-ink-soft"
              data-testid="ghost-explain"
            >
              {t(`ghost.explain.${MODE_KEY[everyone] ?? "precise"}`)}
              {everyone !== geo.GhostMode.PRECISE && everyoneUntil > now && (
                <span className="mt-1 block text-ink">
                  {t("ghost.backIn", { time: compactDuration(everyoneUntil - now) })}
                </span>
              )}
            </p>
          </SheetSection>

          <SheetSection title={t("ghost.timer")}>
            <div
              className="scrollbar-none flex gap-1.5 overflow-x-auto"
              role="radiogroup"
              aria-label={t("ghost.timer")}
            >
              {TIMERS.map((option) => (
                // biome-ignore lint/a11y/useSemanticElements: tappable chips, not radio inputs.
                <button
                  key={option.key}
                  type="button"
                  role="radio"
                  aria-checked={timer === option.seconds}
                  onClick={() => setTimer(option.seconds)}
                  className={cn(
                    "pressable h-9 shrink-0 rounded-full px-3.5 text-xs font-black",
                    timer === option.seconds ? "sticker-sm bg-brand text-on-brand" : "bg-surface-2",
                  )}
                >
                  {t(`ghost.timers.${option.key}`)}
                </button>
              ))}
            </div>
            <p className="mt-1.5 text-[11px] font-bold text-ink-faint">{t("ghost.timerHint")}</p>
          </SheetSection>

          <SheetSection title={t("ghost.perFriend")}>
            {Object.keys(friends).length > 6 && (
              <TextField
                className="mb-2"
                aria-label={t("list.search")}
                placeholder={t("list.search")}
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                trailing={<Search className="mr-3 size-4 text-ink-faint" />}
              />
            )}
            {rows.length === 0 ? (
              <p className="py-4 text-center text-sm text-ink-soft">{t("ghost.noFriends")}</p>
            ) : (
              <ul className="space-y-1">
                {rows.map((friend) => {
                  const userId = friend.user?.id ?? "";
                  const rule = ghostRuleFor(settings, userId, now);
                  const until = toMillis(settings?.overrides[userId]?.until);
                  return (
                    <li
                      key={userId}
                      className="flex items-center gap-2 rounded-2xl px-2 py-1.5"
                      data-testid="ghost-friend-row"
                    >
                      <Avatar user={friend.user} size="sm" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-bold">
                          {friendName(friend, friend.user)}
                        </span>
                        {rule !== everyone && (
                          <span className="block truncate text-[11px] font-bold text-brand">
                            {t("ghost.exception")}
                            {until > now &&
                              ` · ${t("ghost.timeLeft", { time: compactDuration(until - now) })}`}
                          </span>
                        )}
                      </span>
                      <span
                        className="flex gap-0.5 rounded-full bg-surface-2 p-0.5"
                        role="radiogroup"
                        aria-label={t("ghost.forThisFriend", {
                          name: friendName(friend, friend.user),
                        })}
                      >
                        {GHOST_MODES.map(({ mode, key, emoji }) => (
                          // biome-ignore lint/a11y/useSemanticElements: a compact three-way toggle.
                          <button
                            key={key}
                            type="button"
                            role="radio"
                            aria-checked={rule === mode}
                            aria-label={t(`ghost.mode.${key}`)}
                            title={t(`ghost.mode.${key}`)}
                            onClick={() => rule !== mode && setFor.mutate({ userId, mode })}
                            className={cn(
                              "grid size-8 place-items-center rounded-full text-base",
                              rule === mode && "sticker-sm bg-brand",
                            )}
                          >
                            <span aria-hidden>{emoji}</span>
                          </button>
                        ))}
                      </span>
                    </li>
                  );
                })}
              </ul>
            )}
          </SheetSection>
        </>
      )}
    </MapSheet>
  );
}
