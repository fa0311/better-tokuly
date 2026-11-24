import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import { Search } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { ListRow, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";

/** A person in one of the social lists: avatar and name lead to their profile. */
export function UserRow({
  user,
  subtitle,
  trailing,
  testId,
}: {
  user: common.UserSummary;
  subtitle?: ReactNode;
  trailing?: ReactNode;
  testId?: string;
}) {
  return (
    <div data-testid={testId}>
      <ListRow
        leading={
          <Link to="/u/$handle" params={{ handle: user.handle }} tabIndex={-1}>
            <Avatar user={user} outlined />
          </Link>
        }
        title={
          <Link to="/u/$handle" params={{ handle: user.handle }}>
            <EmojiText text={user.displayName} />
          </Link>
        }
        subtitle={subtitle ?? `@${user.handle}`}
        trailing={trailing}
      />
    </div>
  );
}

export function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2">
      {[0, 1, 2].map((i) => (
        <Skeleton key={i} className="h-16" />
      ))}
    </div>
  );
}

export function SearchInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange(value: string): void;
  placeholder: string;
}) {
  return (
    <label className="flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
      <Search className="size-5 text-ink-faint" />
      <input
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={placeholder}
        className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
        autoCapitalize="none"
        spellCheck={false}
      />
    </label>
  );
}

/** `value` once it has stopped changing for a moment (trimmed), for search-as-you-type. */
export function useDebounced(value: string, ms = 300): string {
  const [debounced, setDebounced] = useState(value.trim());
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value.trim()), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

/** A small label next to a name ("フォローされています", "友達"). */
export function Tag({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded-full bg-surface-2 px-2 text-[11px] font-black text-ink-soft",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function SectionTitle({ children }: { children: ReactNode }) {
  return <h2 className="px-3 pb-1 text-sm font-black text-ink-soft">{children}</h2>;
}
