import type { common } from "@tokuly/api";
import { Check, Plus, Search, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { nameOf, useFriends } from "./api";

export interface FriendPickerProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  description?: string;
  /** Who is ticked when the sheet opens. */
  initial?: string[];
  /** Friends left out of the list (already invited, already in the plan). */
  exclude?: string[];
  /** Choose exactly one person: tapping a row confirms. */
  single?: boolean;
  confirmLabel?: string;
  loading?: boolean;
  onConfirm(users: common.UserSummary[]): void;
}

/** A sheet for choosing among the person's friends: search, tick, confirm. */
export function FriendPicker({
  open,
  onOpenChange,
  title,
  description,
  initial,
  exclude,
  single,
  confirmLabel,
  loading,
  onConfirm,
}: FriendPickerProps) {
  const { t } = useTranslation("plans");
  const friends = useFriends();
  const [selected, setSelected] = useState<string[]>(initial ?? []);
  const [query, setQuery] = useState("");
  // Each opening starts from what the caller says is chosen.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setSelected(initial ?? []);
      setQuery("");
    }
  }

  const excluded = new Set(exclude ?? []);
  const users = (friends.data ?? []).flatMap((friend) =>
    friend.user && !excluded.has(friend.user.id) ? [friend.user] : [],
  );
  const needle = query.trim().toLowerCase();
  const shown = needle
    ? users.filter(
        (user) =>
          user.displayName.toLowerCase().includes(needle) ||
          user.handle.toLowerCase().includes(needle),
      )
    : users;
  const toggle = (user: common.UserSummary) => {
    if (single) return onConfirm([user]);
    setSelected((current) =>
      current.includes(user.id) ? current.filter((id) => id !== user.id) : [...current, user.id],
    );
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      footer={
        single ? undefined : (
          <Button
            block
            loading={loading}
            disabled={friends.isPending}
            onClick={() => onConfirm(users.filter((user) => selected.includes(user.id)))}
          >
            {confirmLabel ?? t("picker.confirm")}
            {selected.length > 0 && (
              <span className="rounded-full bg-on-brand/25 px-2 text-sm">{selected.length}</span>
            )}
          </Button>
        )
      }
    >
      {friends.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
        </div>
      ) : users.length === 0 ? (
        <EmptyState emoji="🫥" title={t("picker.empty")} body={t("picker.emptyBody")} />
      ) : (
        <>
          {users.length > 6 && (
            <TextField
              className="mb-3"
              aria-label={t("picker.search")}
              placeholder={t("picker.search")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              trailing={<Search className="mr-2 size-5 text-ink-faint" />}
            />
          )}
          <ul className="flex flex-col gap-1">
            {shown.map((user) => {
              const checked = selected.includes(user.id);
              return (
                <li key={user.id}>
                  <button
                    type="button"
                    {...(single ? {} : { role: "checkbox", "aria-checked": checked })}
                    onClick={() => toggle(user)}
                    className={cn(
                      "flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition-colors hover:bg-surface-2",
                      checked && "bg-brand-soft",
                    )}
                  >
                    <Avatar user={user} outlined />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold">{nameOf(user)}</span>
                      <span className="block truncate text-sm text-ink-soft">@{user.handle}</span>
                    </span>
                    {!single && (
                      <span
                        className={cn(
                          "grid size-7 shrink-0 place-items-center rounded-full border-2",
                          checked ? "border-ink bg-lime text-[#17122b]" : "border-line",
                        )}
                      >
                        {checked && <Check className="size-4" strokeWidth={3} />}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
          {shown.length === 0 && (
            <p className="py-8 text-center text-sm font-bold text-ink-faint">
              {t("picker.noMatch")}
            </p>
          )}
        </>
      )}
    </Dialog>
  );
}

/**
 * The people chosen in a form: their faces as removable chips and a button
 * that opens the picker to change them.
 */
export function FriendChips({
  users,
  onChange,
  label,
  addLabel,
  pickerTitle,
  exclude,
}: {
  users: common.UserSummary[];
  onChange(users: common.UserSummary[]): void;
  label?: string;
  addLabel: string;
  pickerTitle: string;
  exclude?: string[];
}) {
  const { t } = useTranslation("plans");
  const [picking, setPicking] = useState(false);
  return (
    <div className="flex flex-col gap-1.5">
      {label && <p className="px-1 text-sm font-bold text-ink-soft">{label}</p>}
      <div className="flex flex-wrap items-center gap-2">
        {users.map((user) => (
          <span
            key={user.id}
            className="inline-flex h-10 max-w-full items-center gap-2 rounded-full border-2 border-line bg-surface pr-1 pl-1.5"
          >
            <Avatar user={user} size="xs" />
            <span className="truncate text-sm font-bold">{nameOf(user)}</span>
            <button
              type="button"
              aria-label={t("picker.remove", { name: nameOf(user) })}
              onClick={() => onChange(users.filter((other) => other.id !== user.id))}
              className="grid size-7 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
            >
              <X className="size-4" />
            </button>
          </span>
        ))}
        <button
          type="button"
          onClick={() => setPicking(true)}
          className="inline-flex h-10 items-center gap-1.5 rounded-full border-2 border-dashed border-ink-faint px-4 text-sm font-bold text-ink-soft hover:border-ink hover:text-ink"
        >
          <Plus className="size-4" />
          {addLabel}
        </button>
      </div>
      <FriendPicker
        open={picking}
        onOpenChange={setPicking}
        title={pickerTitle}
        initial={users.map((user) => user.id)}
        exclude={exclude}
        onConfirm={(chosen) => {
          onChange(chosen);
          setPicking(false);
        }}
      />
    </div>
  );
}
