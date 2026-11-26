import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { common } from "@tokuly/api";
import { emojiKeys } from "@tokuly/core/emoji";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { TextField } from "#/components/ui/field";
import { Spinner } from "#/components/ui/spinner";
import { profileClient } from "#/lib/api";
import { emojiAdminClient, emojiClient } from "../api";
import { syncCatalog } from "../catalog";
import { useDebounced } from "../parts";

/** After a moderator changed something: every staff list is read again and this browser's catalogue follows. */
export function useAdminRefresh(): () => Promise<void> {
  const queryClient = useQueryClient();
  return async () => {
    void syncCatalog();
    await queryClient.invalidateQueries({ queryKey: emojiKeys.all });
  };
}

export const categoriesKey = ["emoji", "admin", "categories"] as const;

/** Every category in display order, also the empty ones (the catalogue's own list serves the pickers). */
export function useCategories() {
  return useQuery({ queryKey: categoriesKey, queryFn: () => emojiClient().listCategories({}) });
}

export { emojiAdminClient };

export function UserLine({ user }: { user: common.UserSummary | undefined }) {
  const { t } = useTranslation("emoji");
  if (!user || user.deleted) return <span className="text-ink-faint">{t("detail.noAuthor")}</span>;
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 align-middle">
      <Avatar user={user} size="xs" />
      <span className="truncate font-bold">{user.displayName}</span>
      <span className="truncate text-ink-faint">@{user.handle}</span>
    </span>
  );
}

/** Finds an account by name or handle and hands it to `onPick`. */
export function UserSearch({
  label,
  onPick,
  excludeIds = [],
}: {
  label: string;
  onPick(user: common.UserSummary): void;
  excludeIds?: readonly string[];
}) {
  const { t } = useTranslation("emoji");
  const [text, setText] = useState("");
  const query = useDebounced(text.trim(), 250);
  const results = useQuery({
    queryKey: ["identity", "users", "search", query],
    queryFn: async () =>
      (await profileClient().searchUsers({ query, page: { limit: 8 } })).results.flatMap(
        (result) => (result.user ? [result.user] : []),
      ),
    enabled: query.length > 0,
  });
  const users = (results.data ?? []).filter((user) => !excludeIds.includes(user.id));
  return (
    <div>
      <TextField
        label={label}
        value={text}
        onChange={(event) => setText(event.target.value)}
        placeholder={t("admin.userSearch")}
        trailing={
          results.isFetching ? <Spinner className="mr-2 size-4 text-ink-faint" /> : undefined
        }
      />
      {query && (
        <ul className="mt-1.5 flex flex-col rounded-2xl border-2 border-line bg-surface p-1">
          {users.map((user) => (
            <li key={user.id}>
              <button
                type="button"
                onClick={() => {
                  onPick(user);
                  setText("");
                }}
                className="flex w-full items-center gap-2 rounded-xl px-2.5 py-2 text-left text-sm hover:bg-surface-2"
              >
                <UserLine user={user} />
              </button>
            </li>
          ))}
          {results.isSuccess && users.length === 0 && (
            <li className="px-3 py-2 text-sm text-ink-faint">{t("admin.userNone")}</li>
          )}
        </ul>
      )}
    </div>
  );
}
