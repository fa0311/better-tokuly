import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, identity, toMillis } from "@tokuly/api";
import { Search, UserPlus, VolumeX } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { ListRow, Skeleton } from "#/components/ui/page";
import { profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { notifyKeys, preferencesClient } from "../api";
import { formatWhen, useNow } from "../when";

function useMutedUsers() {
  return useInfiniteQuery({
    queryKey: notifyKeys.muted,
    queryFn: ({ pageParam }) => preferencesClient().listMutedUsers({ page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** People whose notifications never arrive, with a way to add and remove them. */
export function MutedUsersCard() {
  const { t, i18n } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const now = useNow();
  const muted = useMutedUsers();
  const [adding, setAdding] = useState(false);
  const users = muted.data?.pages.flatMap((page) => page.users) ?? [];

  const unmute = useMutation({
    mutationFn: (user: common.UserSummary) => preferencesClient().unmuteUser({ userId: user.id }),
    onSuccess: (_, user) => {
      toast.success(t("muted.toast.unmuted", { name: user.displayName }));
      return queryClient.invalidateQueries({ queryKey: notifyKeys.muted });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Card className="px-3 sm:px-5">
      <div className="flex items-start gap-3 px-2 sm:px-0">
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-black leading-snug">{t("muted.title")}</h2>
          <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("muted.lead")}</p>
        </div>
        <Button variant="secondary" size="sm" className="shrink-0" icon={<UserPlus className="size-4" />} onClick={() => setAdding(true)}>
          {t("muted.add")}
        </Button>
      </div>

      <div className="mt-3">
        {muted.isPending ? (
          <Skeleton className="h-14" />
        ) : users.length === 0 ? (
          <p className="flex items-center gap-2 px-2 py-3 text-sm text-ink-faint sm:px-0">
            <VolumeX className="size-4 shrink-0" />
            {t("muted.empty")}
          </p>
        ) : (
          <ul className="sm:-mx-3">
            {users.map(
              ({ user, mutedAt }) =>
                user && (
                  <li key={user.id}>
                    <UserRow
                      user={user}
                      subtitle={`@${user.handle} ・ ${t("muted.since", { when: formatWhen(toMillis(mutedAt), now, i18n.language) })}`}
                      trailing={
                        <Button variant="secondary" size="sm" disabled={unmute.isPending} onClick={() => unmute.mutate(user)} aria-label={t("muted.unmuteOf", { name: user.displayName })}>
                          {t("muted.unmute")}
                        </Button>
                      }
                    />
                  </li>
                ),
            )}
          </ul>
        )}
        <LoadMore query={muted} />
      </div>

      {adding && <MuteDialog mutedIds={new Set(users.map(({ user }) => user?.id ?? ""))} onClose={() => setAdding(false)} />}
    </Card>
  );
}

function UserRow({ user, subtitle, trailing }: { user: common.UserSummary; subtitle: ReactNode; trailing: ReactNode }) {
  return (
    <ListRow
      leading={
        <Link to="/u/$handle" params={{ handle: user.handle }}>
          <Avatar user={user} outlined />
        </Link>
      }
      title={user.displayName}
      subtitle={subtitle}
      trailing={trailing}
    />
  );
}

/** Search for someone and mute them. Stays open so several people can be muted in one go. */
function MuteDialog({ mutedIds, onClose }: { mutedIds: ReadonlySet<string>; onClose(): void }) {
  const { t } = useTranslation("notifications");
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query.trim()), 300);
    return () => clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["identity", "search", debounced],
    queryFn: () => profileClient().searchUsers({ query: debounced }),
    enabled: debounced.length > 0,
  });
  const people = results.data?.results.filter((result) => result.user && result.relationship !== identity.Relationship.SELF) ?? [];

  const mute = useMutation({
    mutationFn: (user: common.UserSummary) => preferencesClient().muteUser({ userId: user.id }),
    onSuccess: (_, user) => {
      toast.success(t("muted.toast.muted", { name: user.displayName }));
      return queryClient.invalidateQueries({ queryKey: notifyKeys.muted });
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("muted.dialog.title")} description={t("muted.dialog.lead")}>
      <label className="flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
        <Search className="size-5 shrink-0 text-ink-faint" />
        <input
          // The dialog exists to type a name into this field.
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("muted.dialog.placeholder")}
          aria-label={t("muted.dialog.placeholder")}
          className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
          autoCapitalize="none"
          spellCheck={false}
        />
      </label>
      <div className="mt-3 min-h-40">
        {!debounced ? (
          <p className="px-2 py-8 text-center text-sm text-ink-faint">{t("muted.dialog.hint")}</p>
        ) : results.isPending ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-14" />
            <Skeleton className="h-14" />
          </div>
        ) : people.length === 0 ? (
          <p className="px-2 py-8 text-center text-sm text-ink-faint">{t("muted.dialog.empty")}</p>
        ) : (
          <ul className="-mx-3">
            {people.map(
              ({ user }) =>
                user && (
                  <li key={user.id}>
                    <ListRow
                      leading={<Avatar user={user} outlined />}
                      title={user.displayName}
                      subtitle={`@${user.handle}`}
                      trailing={
                        mutedIds.has(user.id) ? (
                          <Chip>{t("muted.dialog.already")}</Chip>
                        ) : (
                          <Button size="sm" icon={<VolumeX className="size-4" />} disabled={mute.isPending} onClick={() => mute.mutate(user)} aria-label={t("muted.muteOf", { name: user.displayName })}>
                            {t("muted.mute")}
                          </Button>
                        )
                      }
                    />
                  </li>
                ),
            )}
          </ul>
        )}
      </div>
    </Dialog>
  );
}
