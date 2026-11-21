import { Search, X } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "#/lib/cn";

export interface FilterOption<T extends string> {
  value: T;
  label: string;
}

const control = "h-10 w-full rounded-control border-2 border-line bg-surface text-sm font-bold text-ink focus:border-brand focus:outline-none";

const caption = "px-1 text-[11px] font-bold text-ink-faint";

/** A caption above a control that labels itself (a user picker) in a filter row. */
export function FilterField({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className={caption}>{label}</span>
      {children}
    </div>
  );
}

/** A compact dropdown for filter rows (the big `SelectField` is for settings). */
export function FilterSelect<T extends string>({ label, value, onChange, options, className }: { label: string; value: T; onChange(value: T): void; options: FilterOption<T>[]; className?: string }) {
  return (
    <label className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className={caption}>{label}</span>
      <select value={value} onChange={(event) => onChange(event.target.value as T)} className={cn(control, "px-3")}>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

/** A plain text or date-time input in a filter row. */
export function FilterInput({ label, value, onChange, type = "text", placeholder, className }: { label: string; value: string; onChange(value: string): void; type?: "text" | "datetime-local"; placeholder?: string; className?: string }) {
  return (
    <label className={cn("flex min-w-0 flex-col gap-1", className)}>
      <span className={caption}>{label}</span>
      <input type={type} value={value} onChange={(event) => onChange(event.target.value)} placeholder={placeholder} className={cn(control, "px-3 font-medium placeholder:text-ink-faint")} autoCapitalize="none" spellCheck={false} />
    </label>
  );
}

/** The search field at the top of a list. */
export function SearchBox({ label, value, onChange, placeholder, className }: { label: string; value: string; onChange(value: string): void; placeholder?: string; className?: string }) {
  return (
    <label className={cn("flex h-10 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand", className)}>
      <Search className="size-4 shrink-0 text-ink-faint" />
      <input
        type="search"
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-ink-faint [&::-webkit-search-cancel-button]:hidden"
        autoCapitalize="none"
        spellCheck={false}
      />
    </label>
  );
}

/** A chosen filter value that can be removed (a picked user, a pinned ID). */
export function FilterToken({ label, children, onClear, clearLabel }: { label: string; children: ReactNode; onClear(): void; clearLabel: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className={caption}>{label}</span>
      <span className="flex h-10 min-w-0 items-center gap-1 rounded-control border-2 border-brand bg-brand-soft pr-1 pl-3 text-sm">
        <span className="min-w-0 flex-1 truncate">{children}</span>
        <button type="button" aria-label={clearLabel} onClick={onClear} className="grid size-7 shrink-0 place-items-center rounded-full hover:bg-surface">
          <X className="size-4" />
        </button>
      </span>
    </div>
  );
}
