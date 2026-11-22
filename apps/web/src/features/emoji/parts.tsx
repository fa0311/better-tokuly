import { Link } from "@tanstack/react-router";
import { emoji as emojiApi } from "@tokuly/api";
import { Search, X } from "lucide-react";
import { type CSSProperties, type ReactNode, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { emojiPageParam } from "./api";
import type { CatalogEmoji } from "./catalog";
import { EmojiImage } from "./emoji";

/** A custom emoji in a grid: its picture over its shortcode, linking to its page. */
export function EmojiTile({ emoji }: { emoji: CatalogEmoji }) {
  return (
    <Link
      to="/emoji/$name"
      params={{ name: emojiPageParam(emoji.name) }}
      data-testid="emoji-tile"
      className="pressable flex min-w-0 flex-col items-center gap-1.5 rounded-2xl bg-surface px-1.5 pt-3 pb-2 hover:bg-surface-2"
    >
      <EmojiImage emoji={emoji} size={44} className="size-11" />
      <span className="w-full truncate text-center text-[11px] font-bold text-ink-soft">:{emoji.name}:</span>
    </Link>
  );
}

/** The responsive grid emoji tiles sit in. */
export function TileGrid({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("grid grid-cols-4 gap-1.5 sm:grid-cols-6 lg:grid-cols-8 xl:grid-cols-10", className)}>{children}</div>;
}

export function TileSkeletons({ count = 16 }: { count?: number }) {
  return (
    <TileGrid>
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="h-[5.25rem] animate-pulse rounded-2xl bg-surface-3" />
      ))}
    </TileGrid>
  );
}

const STATE_KEYS: Record<emojiApi.EmojiState, string> = {
  [emojiApi.EmojiState.UNSPECIFIED]: "pending",
  [emojiApi.EmojiState.PENDING]: "pending",
  [emojiApi.EmojiState.APPROVED]: "approved",
  [emojiApi.EmojiState.REJECTED]: "rejected",
  [emojiApi.EmojiState.DISABLED]: "disabled",
};

const STATE_TONES: Record<string, string> = {
  pending: "bg-sun text-[#17122b]",
  approved: "bg-lime text-[#17122b]",
  rejected: "bg-danger text-white",
  disabled: "bg-surface-3 text-ink-soft",
};

/** Key of a state's name in the `emoji` strings: `state.<key>`. */
export const stateKey = (state: emojiApi.EmojiState) => STATE_KEYS[state];

/** Where an emoji is in its lifecycle, as a coloured label (the text says it too). */
export function StateChip({ state }: { state: emojiApi.EmojiState }) {
  const { t } = useTranslation("emoji");
  const key = stateKey(state);
  return (
    <Chip data-state={key} className={STATE_TONES[key]}>
      {t(`state.${key}`)}
    </Chip>
  );
}

/** A rounded search box with a clear button. */
export function SearchBox({ value, onChange, placeholder, className, autoFocus }: { value: string; onChange(value: string): void; placeholder: string; className?: string; autoFocus?: boolean }) {
  const { t } = useTranslation("emoji");
  return (
    <label className={cn("flex h-12 items-center gap-2 rounded-full border-2 border-line bg-surface px-4 focus-within:border-brand", className)}>
      <Search className="size-5 shrink-0 text-ink-faint" aria-hidden />
      <input
        type="search"
        // biome-ignore lint/a11y/noAutofocus: only where the caller arrived to type.
        autoFocus={autoFocus}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={placeholder}
        className="min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint [&::-webkit-search-cancel-button]:hidden"
      />
      {value && (
        <button type="button" aria-label={t("picker.clear")} onClick={() => onChange("")} className="grid size-7 place-items-center rounded-full text-ink-faint hover:bg-surface-2">
          <X className="size-4" />
        </button>
      )}
    </label>
  );
}

/** A pill that filters a list; `aria-pressed` says whether it is the chosen one. */
export function FilterPill({ active, onClick, children }: { active: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn("flex h-9 shrink-0 items-center gap-1.5 rounded-full border-2 px-3.5 text-sm font-bold transition-colors", active ? "border-ink bg-brand text-on-brand" : "border-line bg-surface text-ink-soft hover:border-ink-faint")}
    >
      {children}
    </button>
  );
}

/**
 * The surface colours of both themes, as `styles.css` defines them on the
 * root element. The theme is an attribute of <html>, so a tile cannot ask
 * for "the other theme": the preview sets the tokens it needs itself.
 */
const THEME_TOKENS: Record<"light" | "dark", CSSProperties> = {
  light: { "--surface": "#ffffff", "--ink": "#17122b", "--ink-faint": "#9993ad", "--line": "#ddd6ee" } as CSSProperties,
  dark: { "--surface": "#17132a", "--ink": "#f6f3ff", "--ink-faint": "#7a7397", "--line": "#322b52" } as CSSProperties,
};

/** A tile in one theme's colours, whatever theme the page is in. */
export function ThemeSwatch({ theme, className, children }: { theme: "light" | "dark"; className?: string; children: ReactNode }) {
  return (
    <div style={THEME_TOKENS[theme]} data-testid={`preview-${theme}`} className={cn("rounded-2xl border-2 border-line bg-surface text-ink", className)}>
      {children}
    </div>
  );
}

/**
 * The same content on a light and on a dark tile: what an emoji will look
 * like in a message, whatever theme the reader uses.
 */
export function ThemePreview({ children, className }: { children: ReactNode; className?: string }) {
  const { t } = useTranslation("emoji");
  return (
    <div className={cn("grid grid-cols-2 gap-2", className)}>
      {(["light", "dark"] as const).map((theme) => (
        <ThemeSwatch key={theme} theme={theme} className="px-3 py-2.5">
          <p className="text-[10px] font-black text-ink-faint">{t(`preview.${theme}`)}</p>
          <div className="mt-1 text-[15px] leading-relaxed">{children}</div>
        </ThemeSwatch>
      ))}
    </div>
  );
}

/** `value`, a moment after it stopped changing: for searching while typing. */
export function useDebounced<T>(value: T, delay = 250): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return settled;
}
