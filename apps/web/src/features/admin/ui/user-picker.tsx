import { useQuery } from "@tanstack/react-query";
import type { admin } from "@tokuly/api";
import { Search } from "lucide-react";
import { useId, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { userAdminClient } from "../api";
import { useDebounced } from "./hooks";

/** Accounts that may work on cases: admins hold the moderator role implicitly. */
const MODERATING_ROLES = ["moderator", "admin"] as const;

async function searchAccounts(query: string, staffOnly: boolean): Promise<admin.AdminUser[]> {
  if (!staffOnly) return (await userAdminClient().searchUsers({ query, page: { limit: 8 } })).users;
  // The API filters by one role at a time.
  const pages = await Promise.all(
    MODERATING_ROLES.map((role) =>
      userAdminClient().searchUsers({ query, role, page: { limit: 8 } }),
    ),
  );
  const seen = new Set<string>();
  return pages
    .flatMap((page) => page.users)
    .filter((account) => {
      const id = account.user?.id ?? "";
      if (!id || seen.has(id)) return false;
      seen.add(id);
      return true;
    });
}

/**
 * Finds an account by name, handle, email or ID and hands it to `onPick`.
 * Used wherever the console asks "which user?": assignees, allow-lists,
 * filters, flag previews.
 */
export function UserPicker({
  label,
  placeholder,
  onPick,
  staffOnly = false,
  excludeIds,
  className,
}: {
  label: string;
  placeholder?: string;
  onPick(account: admin.AdminUser): void;
  /** Only moderators and admins (case assignees). */
  staffOnly?: boolean;
  /** Accounts already chosen, left out of the results. */
  excludeIds?: readonly string[];
  className?: string;
}) {
  const { t } = useTranslation("admin");
  const listId = useId();
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const query = useDebounced(text.trim(), 250);
  const results = useQuery({
    queryKey: ["admin", "users", "picker", staffOnly, query],
    queryFn: () => searchAccounts(query, staffOnly),
    enabled: open,
  });
  const accounts = (results.data ?? []).filter(
    (account) => account.user && !excludeIds?.includes(account.user.id),
  );

  const pick = (account: admin.AdminUser) => {
    onPick(account);
    setText("");
    setOpen(false);
  };

  return (
    <div className={cn("relative", className)}>
      <label className="flex h-10 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand">
        <Search className="size-4 shrink-0 text-ink-faint" />
        <input
          role="combobox"
          aria-label={label}
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
            if (event.key === "Enter" && open && accounts[0]) {
              event.preventDefault();
              pick(accounts[0]);
            }
          }}
          placeholder={placeholder ?? t("common.userSearchPlaceholder")}
          className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        {results.isFetching && open && <Spinner className="size-4 text-ink-faint" />}
      </label>
      {open && (
        <div
          id={listId}
          role="listbox"
          aria-label={label}
          className="sticker-sm absolute inset-x-0 top-full z-30 mt-1.5 max-h-72 overflow-y-auto rounded-2xl bg-surface p-1.5"
        >
          {accounts.length === 0 ? (
            <p className="px-3 py-2.5 text-sm text-ink-faint">
              {results.isPending ? t("common.searching") : t("common.noUsers")}
            </p>
          ) : (
            accounts.map(
              (account) =>
                account.user && (
                  <button
                    key={account.user.id}
                    type="button"
                    role="option"
                    aria-selected={false}
                    // Keep the input focused so the list is still open when the click lands.
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => pick(account)}
                    className="flex w-full items-center gap-2.5 rounded-xl px-2.5 py-2 text-left hover:bg-surface-2"
                  >
                    <Avatar user={account.user} size="sm" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-bold">
                        {account.user.displayName || account.user.handle}
                      </span>
                      <span className="block truncate text-xs text-ink-soft">
                        @{account.user.handle}
                        {account.email && ` · ${account.email}`}
                      </span>
                    </span>
                  </button>
                ),
            )
          )}
        </div>
      )}
    </div>
  );
}
