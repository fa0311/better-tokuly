import type { common } from "@tokuly/api";
import { Check, X } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { SearchField } from "./parts";
import { displayName, useFriends } from "./people";

/**
 * Picks several friends: a search, the friends as rows to tick, and the
 * chosen ones as chips on top. `excludeIds` are left out (people who are in
 * the group already).
 */
export function FriendPicker({
  selected,
  onChange,
  excludeIds = [],
  limit,
}: {
  selected: common.UserSummary[];
  onChange(selected: common.UserSummary[]): void;
  excludeIds?: string[];
  /** At most this many can be chosen. */
  limit?: number;
}) {
  const { t } = useTranslation("chat");
  const [query, setQuery] = useState("");
  const friends = useFriends(query.trim().replace(/^@/, ""));
  const candidates = (friends.data?.friends ?? []).flatMap((friend) =>
    friend.user && !excludeIds.includes(friend.user.id) ? [friend.user] : [],
  );
  const isSelected = (user: common.UserSummary) => selected.some((chosen) => chosen.id === user.id);
  const full = limit !== undefined && selected.length >= limit;
  const toggle = (user: common.UserSummary) => {
    if (isSelected(user)) onChange(selected.filter((chosen) => chosen.id !== user.id));
    else if (!full) onChange([...selected, user]);
  };

  return (
    <div className="flex flex-col gap-3">
      {selected.length > 0 && (
        <ul className="flex flex-wrap gap-1.5" aria-label={t("picker.selected")}>
          {selected.map((user) => (
            <li key={user.id}>
              <button
                type="button"
                onClick={() => toggle(user)}
                className="flex h-8 items-center gap-1.5 rounded-full bg-brand-soft pr-2 pl-1 text-sm font-bold"
                aria-label={t("picker.remove", { name: displayName(user) })}
              >
                <Avatar user={user} size="xs" />
                <span className="max-w-28 truncate">
                  <EmojiText text={displayName(user)} />
                </span>
                <X className="size-3.5 text-ink-soft" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <SearchField value={query} onChange={setQuery} placeholder={t("picker.search")} />
      {friends.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2].map((index) => (
            <Skeleton key={index} className="h-14" />
          ))}
        </div>
      ) : candidates.length === 0 ? (
        <EmptyState
          emoji="🫥"
          title={query ? t("picker.noMatch") : t("picker.noFriends")}
          body={query ? undefined : t("picker.noFriendsBody")}
        />
      ) : (
        <ul className="-mx-2 flex flex-col">
          {candidates.map((user) => {
            const chosen = isSelected(user);
            return (
              <li key={user.id}>
                <button
                  type="button"
                  aria-pressed={chosen}
                  disabled={!chosen && full}
                  onClick={() => toggle(user)}
                  className="flex w-full items-center gap-3 rounded-2xl px-3 py-2 text-left transition-colors hover:bg-surface-2 disabled:opacity-50"
                >
                  <Avatar user={user} outlined />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-bold">
                      <EmojiText text={displayName(user)} />
                    </span>
                    <span className="block truncate text-sm text-ink-soft">@{user.handle}</span>
                  </span>
                  <span
                    className={cn(
                      "grid size-6 shrink-0 place-items-center rounded-full border-2",
                      chosen ? "border-ink bg-lime text-[#17122b]" : "border-line",
                    )}
                  >
                    {chosen && <Check className="size-4" strokeWidth={3} />}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
