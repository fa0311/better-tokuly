import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { Popover } from "@base-ui/react/popover";
import type { emoji as emojiApi } from "@tokuly/api";
import {
  type Catalog,
  type CatalogEmoji,
  customChoice,
  type EmojiChoice,
  searchEmoji,
  toCatalogEmoji,
} from "@tokuly/core/emoji";
import { Search, Star, X } from "lucide-react";
import {
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useDeck, useRecentEmoji } from "./api";
import { useCatalog } from "./catalog";
import { EmojiImage } from "./emoji";
import { recordEmojiUse, useUnicodeTable } from "./use";

export interface EmojiPickerProps {
  /**
   * Called with the emoji the person chose, written as it goes into text and
   * reactions: `:name:` for a custom emoji, the character for a Unicode one.
   * `choice` has the details (the custom emoji's picture, its name).
   */
  onPick(emoji: string, choice: EmojiChoice): void;
  /** The element that opens the picker. It must forward its props and ref, which `Button` and plain DOM elements do. */
  trigger: ReactElement<Record<string, unknown>>;
  /** Where the panel opens relative to the trigger on wide screens. Default: above it. */
  side?: "top" | "bottom";
  /** Keep the picker open after a pick, to put several emoji into a text. Default: it closes. */
  keepOpen?: boolean;
  /** Told when the picker opens and closes. */
  onOpenChange?(open: boolean): void;
  /**
   * Offer Unicode emoji only: for a field or an RPC that does not take
   * `:shortcode:`s (a field that holds one Unicode emoji).
   */
  unicodeOnly?: boolean;
  /** Leave out custom emoji marked sensitive: for a place that does not take them. */
  hideSensitive?: boolean;
  /**
   * Offer only these emoji (as written in text: `:name:` or the character),
   * in this order, without tabs: a reaction on a subject that carries as many
   * different emoji as it may can only join one that is there.
   */
  only?: string[];
}

/** What a picker may offer, from its props. */
type Offer = Pick<EmojiPickerProps, "unicodeOnly" | "hideSensitive" | "only">;

/**
 * The inside of the picker (search, tabs, grids) for a surface that brings
 * its own frame, such as a dialog opened from a menu. It fills its parent,
 * which must be a column with a height:
 *
 *   <div className="flex h-96 flex-col"><EmojiPickerPanel onPick={(emoji) => react(emoji)} unicodeOnly /></div>
 */
export function EmojiPickerPanel({
  onPick,
  autoFocusSearch = false,
  ...offer
}: Pick<EmojiPickerProps, "onPick"> & Offer & { autoFocusSearch?: boolean }) {
  return (
    <PickerPanel
      autoFocusSearch={autoFocusSearch}
      {...offer}
      onPick={(choice) => {
        recordEmojiUse([choice.emoji]);
        onPick(choice.emoji, choice);
      }}
    />
  );
}

/**
 * The emoji picker, Unicode and custom emoji together: the person's reaction
 * deck first, then what they used recently, the custom categories and the
 * Unicode groups; one search over all of them by name and alias in Japanese
 * and English. A sheet rising from the bottom on phones, a panel floating
 * next to its trigger on wider screens. Arrow keys move through the emoji,
 * Enter picks.
 *
 *   <EmojiPicker trigger={<Button size="icon" aria-label="絵文字">…</Button>} onPick={(emoji) => insert(emoji)} />
 */
export function EmojiPicker({
  onPick,
  trigger,
  side = "top",
  keepOpen,
  onOpenChange,
  ...offer
}: EmojiPickerProps) {
  const { t } = useTranslation("emoji");
  const [open, setOpenState] = useState(false);
  const wide = useMediaQuery("(min-width: 640px)");
  const popup = useRef<HTMLDivElement>(null);
  const setOpen = (next: boolean) => {
    setOpenState(next);
    onOpenChange?.(next);
  };
  const panel = open && (
    <PickerPanel
      autoFocusSearch={wide}
      {...offer}
      onPick={(choice) => {
        recordEmojiUse([choice.emoji]);
        if (!keepOpen) setOpen(false);
        onPick(choice.emoji, choice);
      }}
    />
  );

  if (wide) {
    return (
      <Popover.Root open={open} onOpenChange={setOpen}>
        <Popover.Trigger render={trigger} />
        <Popover.Portal>
          <Popover.Positioner
            side={side}
            align="center"
            sideOffset={8}
            collisionPadding={10}
            className="z-50 outline-none"
          >
            <Popover.Popup
              ref={popup}
              aria-label={t("picker.title")}
              data-testid="emoji-picker"
              className={cn(
                "sticker-lg flex h-[26rem] max-h-[var(--available-height)] w-[22.5rem] origin-[var(--transform-origin)] flex-col overflow-hidden rounded-[1.5rem] bg-surface outline-none",
                "transition-[transform,opacity] duration-150 data-[ending-style]:scale-95 data-[ending-style]:opacity-0 data-[starting-style]:scale-95 data-[starting-style]:opacity-0",
              )}
            >
              {panel}
            </Popover.Popup>
          </Popover.Positioner>
        </Popover.Portal>
      </Popover.Root>
    );
  }
  return (
    <BaseDialog.Root open={open} onOpenChange={setOpen}>
      <BaseDialog.Trigger render={trigger} />
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-50 bg-overlay transition-opacity duration-200 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0" />
        <BaseDialog.Popup
          ref={popup}
          // Focus goes to the sheet, not into its search box: that would raise the keyboard over the emoji.
          initialFocus={popup}
          data-testid="emoji-picker"
          className={cn(
            "sticker-lg fixed inset-x-0 bottom-0 z-50 flex h-[min(28rem,70dvh)] flex-col overflow-hidden rounded-t-[2rem] bg-surface pb-safe outline-none",
            "transition-transform duration-300 ease-out-soft data-[ending-style]:translate-y-full data-[starting-style]:translate-y-full",
          )}
        >
          <BaseDialog.Title className="sr-only">{t("picker.title")}</BaseDialog.Title>
          <span
            aria-hidden
            className="mx-auto mt-2 h-1.5 w-10 shrink-0 rounded-full bg-surface-3"
          />
          {panel}
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (notify) => {
      const list = window.matchMedia(query);
      list.addEventListener("change", notify);
      return () => list.removeEventListener("change", notify);
    },
    () => window.matchMedia(query).matches,
    () => false,
  );
}

/** Deck and recently used items as choices; custom emoji that left the catalogue are skipped. */
export function toChoices(items: readonly emojiApi.PickerItem[], catalog: Catalog): EmojiChoice[] {
  const choices: EmojiChoice[] = [];
  for (const item of items) {
    if (item.emoji.startsWith(":")) {
      // The local copy first (it may be newer than the list), then what the server attached.
      const custom =
        catalog.byName.get(item.emoji.slice(1, -1)) ??
        (item.custom ? toCatalogEmoji(item.custom) : undefined);
      if (custom) choices.push(customChoice(custom));
    } else if (item.emoji) {
      choices.push({ kind: "unicode", emoji: item.emoji, name: item.emoji });
    }
  }
  return choices;
}

/** Tab of the deck and the recently used emoji. */
const HOME = "home";
/** Tab of the custom emoji without a category. */
const UNCATEGORIZED = "custom:";
/** Cells drawn at first in a long category; more follow as the list is scrolled. */
const CHUNK = 240;

function PickerPanel({
  onPick,
  autoFocusSearch,
  unicodeOnly = false,
  hideSensitive = false,
  only,
}: Offer & {
  onPick(choice: EmojiChoice): void;
  autoFocusSearch: boolean;
}) {
  const { t } = useTranslation("emoji");
  const catalog = useCatalog();
  const unicode = useUnicodeTable();
  const deck = useDeck();
  const recent = useRecentEmoji();
  const [tab, setTab] = useState(HOME);
  const [text, setText] = useState("");
  const [hovered, setHovered] = useState<EmojiChoice | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);

  const query = text.trim();
  const offered = only ? new Set(only) : undefined;
  const custom = unicodeOnly
    ? []
    : catalog.emojis.filter(
        (entry) =>
          !(hideSensitive && entry.sensitive) && (!offered || offered.has(`:${entry.name}:`)),
      );
  const allowed = (choices: EmojiChoice[]) =>
    choices.filter(
      (choice) =>
        (!unicodeOnly || choice.kind === "unicode") &&
        !(hideSensitive && choice.kind === "custom" && choice.custom.sensitive) &&
        (!offered || offered.has(choice.emoji)),
    );
  const results = query ? allowed(searchEmoji(query, custom, unicode?.all ?? [], 96)) : [];
  // With `only`, the emoji named there (custom ones the catalogue still has), in their order.
  const onlyChoices: EmojiChoice[] = allowed(
    (only ?? []).flatMap((emoji): EmojiChoice[] => {
      const name = /^:(.+):$/.exec(emoji)?.[1];
      if (!name) return [{ kind: "unicode", emoji, name: emoji }];
      const entry = catalog.byName.get(name);
      return entry ? [customChoice(entry)] : [];
    }),
  );

  const uncategorized = custom.filter((entry) => !entry.category);
  const customTabs = [
    ...catalog.categories
      .filter((category) => category.emojiCount > 0)
      .map((category) => ({
        id: `custom:${category.name}`,
        label: category.name,
        emojis: custom.filter((entry) => entry.category === category.name),
      })),
    ...(uncategorized.length > 0
      ? [
          {
            id: UNCATEGORIZED,
            label: t(catalog.categories.length > 0 ? "picker.uncategorized" : "picker.custom"),
            emojis: uncategorized,
          },
        ]
      : []),
  ].filter((entry) => entry.emojis.length > 0);

  const cells = (): HTMLElement[] =>
    Array.from(body.current?.querySelectorAll<HTMLElement>("[data-emoji-cell]") ?? []);

  // Arrow keys walk the grid the way it looks: left and right in reading order, up and down to the nearest cell of the next row.
  const onGridKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const all = cells();
    const index = all.indexOf(document.activeElement as HTMLElement);
    if (index < 0) return;
    let target: HTMLElement | undefined;
    if (event.key === "ArrowRight") target = all[index + 1];
    else if (event.key === "ArrowLeft") target = all[index - 1];
    else if (event.key === "Home") target = all[0];
    else if (event.key === "End") target = all[all.length - 1];
    else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const from = (all[index] as HTMLElement).getBoundingClientRect();
      const down = event.key === "ArrowDown";
      let bestRow = Number.POSITIVE_INFINITY;
      let bestColumn = Number.POSITIVE_INFINITY;
      for (const cell of all) {
        const box = cell.getBoundingClientRect();
        const rowDistance = down ? box.top - from.top : from.top - box.top;
        if (rowDistance < 4) continue;
        const columnDistance = Math.abs(box.left - from.left);
        if (
          rowDistance < bestRow - 2 ||
          (Math.abs(rowDistance - bestRow) <= 2 && columnDistance < bestColumn)
        ) {
          bestRow = rowDistance;
          bestColumn = columnDistance;
          target = cell;
        }
      }
      if (!target && !down) {
        event.preventDefault();
        search.current?.focus();
        return;
      }
    } else return;
    event.preventDefault();
    target?.focus();
    target?.scrollIntoView({ block: "nearest" });
  };

  const onSearchKey = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      cells()[0]?.focus();
    } else if (event.key === "Enter" && !event.nativeEvent.isComposing && results[0]) {
      event.preventDefault();
      onPick(results[0]);
    }
  };

  const cellProps = { onPick, onHover: setHovered };
  const deckChoices = allowed(toChoices(deck.data?.items ?? [], catalog));
  const recentChoices = allowed(toChoices(recent.data ?? [], catalog)).slice(0, 24);
  const customTab = customTabs.find((entry) => entry.id === tab);
  const unicodeGroup = unicode?.groups.find((group) => `unicode:${group.id}` === tab);

  return (
    <>
      <div className="shrink-0 px-3 pt-3">
        <label className="flex h-10 items-center gap-2 rounded-full border-2 border-line bg-surface px-3 focus-within:border-brand">
          <Search className="size-4 shrink-0 text-ink-faint" aria-hidden />
          <input
            ref={search}
            type="search"
            // biome-ignore lint/a11y/noAutofocus: opening the picker on a keyboard device is asking to type a name.
            autoFocus={autoFocusSearch}
            value={text}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={onSearchKey}
            placeholder={t("picker.search")}
            aria-label={t("picker.search")}
            className="min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint [&::-webkit-search-cancel-button]:hidden"
          />
          {text && (
            <button
              type="button"
              aria-label={t("picker.clear")}
              onClick={() => setText("")}
              className="grid size-6 place-items-center rounded-full text-ink-faint hover:bg-surface-2"
            >
              <X className="size-4" />
            </button>
          )}
        </label>
      </div>

      {!query && !only && (
        <div
          role="tablist"
          aria-label={t("picker.title")}
          className="scrollbar-none flex shrink-0 gap-0.5 overflow-x-auto px-2 pt-2"
        >
          <TabButton id={HOME} current={tab} onSelect={setTab} label={t("picker.home")}>
            <Star className="size-[18px]" />
          </TabButton>
          {customTabs.map((entry) => (
            <TabButton
              key={entry.id}
              id={entry.id}
              current={tab}
              onSelect={setTab}
              label={entry.label}
            >
              {/* The tab shows an emoji of the category that everyone may see as it is. */}
              <EmojiImage
                emoji={
                  entry.emojis.find((item) => !item.sensitive) ?? (entry.emojis[0] as CatalogEmoji)
                }
                size={22}
                still
                className="size-[22px]"
              />
            </TabButton>
          ))}
          {unicode?.groups.map((group) => (
            <TabButton
              key={group.id}
              id={`unicode:${group.id}`}
              current={tab}
              onSelect={setTab}
              label={t(`picker.groups.${group.id}`)}
            >
              <span aria-hidden className="text-lg leading-none">
                {group.icon}
              </span>
            </TabButton>
          ))}
        </div>
      )}

      {/* The key handler moves focus between the emoji buttons inside. */}
      <div
        ref={body}
        onKeyDown={onGridKey}
        className="min-h-0 flex-1 overflow-y-auto px-2 pt-1 pb-2"
        data-testid="emoji-picker-body"
      >
        {query ? (
          results.length > 0 ? (
            <Grid label={t("picker.results")} choices={results} first {...cellProps} />
          ) : (
            <p className="px-4 py-10 text-center text-sm font-bold text-ink-soft">
              {t(unicode && catalog.status !== "loading" ? "picker.noResults" : "picker.loading")}
            </p>
          )
        ) : only ? (
          <Grid label={t("picker.onlyThese")} choices={onlyChoices} first {...cellProps} />
        ) : tab === HOME ? (
          <>
            <Grid label={t("picker.deck")} choices={deckChoices} first {...cellProps} />
            {recentChoices.length > 0 && (
              <Grid label={t("picker.recent")} choices={recentChoices} {...cellProps} />
            )}
          </>
        ) : customTab ? (
          <GrowingGrid
            key={customTab.id}
            label={customTab.label}
            choices={customTab.emojis.map(customChoice)}
            {...cellProps}
          />
        ) : unicodeGroup ? (
          <Grid
            label={t(`picker.groups.${unicodeGroup.id}`)}
            choices={allowed(
              unicodeGroup.emoji.map((entry) => ({
                kind: "unicode" as const,
                emoji: entry.emoji,
                name: entry.en[0] ?? entry.emoji,
              })),
            )}
            first
            {...cellProps}
          />
        ) : null}
      </div>

      <div
        className="flex h-9 shrink-0 items-center gap-2 border-t-2 border-line px-3 text-sm"
        aria-hidden
      >
        {hovered ? (
          <>
            <span className="grid size-6 shrink-0 place-items-center text-lg leading-none">
              {hovered.kind === "custom" ? (
                <EmojiImage emoji={hovered.custom} size={24} still className="size-6" />
              ) : (
                hovered.emoji
              )}
            </span>
            <span className="truncate font-bold">
              {hovered.kind === "custom"
                ? `:${hovered.name}:`
                : hovered.name === hovered.emoji
                  ? ""
                  : hovered.name}
            </span>
            {hovered.kind === "custom" && hovered.custom.aliases.length > 0 && (
              <span className="truncate text-xs text-ink-faint">
                {hovered.custom.aliases.slice(0, 4).join(" · ")}
              </span>
            )}
          </>
        ) : (
          <span className="truncate text-xs font-bold text-ink-faint">{t("picker.hint")}</span>
        )}
      </div>
    </>
  );
}

function TabButton({
  id,
  current,
  onSelect,
  label,
  children,
}: {
  id: string;
  current: string;
  onSelect(id: string): void;
  label: string;
  children: ReactNode;
}) {
  const active = id === current;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      aria-label={label}
      title={label}
      onClick={() => onSelect(id)}
      className={cn(
        "grid h-9 min-w-10 flex-1 shrink-0 place-items-center rounded-xl transition-colors",
        active
          ? "bg-brand-soft text-brand"
          : "text-ink-soft opacity-70 hover:bg-surface-2 hover:opacity-100",
      )}
    >
      {children}
    </button>
  );
}

interface CellHandlers {
  onPick(choice: EmojiChoice): void;
  onHover(choice: EmojiChoice | null): void;
}

function Grid({
  label,
  choices,
  first,
  onPick,
  onHover,
  children,
}: CellHandlers & {
  label: string;
  choices: EmojiChoice[] /** Its first emoji is where Tab lands. */;
  first?: boolean;
  children?: ReactNode;
}) {
  return (
    <section aria-label={label}>
      <h3 className="px-1.5 pt-2 pb-1 text-xs font-black text-ink-faint">{label}</h3>
      <div className="grid grid-cols-8 gap-0.5">
        {choices.map((choice, index) => (
          <button
            key={choice.emoji}
            type="button"
            data-emoji-cell
            tabIndex={first && index === 0 ? 0 : -1}
            aria-label={
              choice.kind === "custom"
                ? choice.emoji
                : `${choice.emoji} ${choice.name === choice.emoji ? "" : choice.name}`.trim()
            }
            onClick={() => onPick(choice)}
            onPointerEnter={() => onHover(choice)}
            onFocus={() => onHover(choice)}
            className="grid aspect-square place-items-center rounded-xl text-[1.4rem] leading-none transition-transform hover:bg-surface-2 focus-visible:bg-surface-2 active:scale-125"
          >
            {choice.kind === "custom" ? (
              <EmojiImage emoji={choice.custom} size={28} className="size-7" />
            ) : (
              <span aria-hidden>{choice.emoji}</span>
            )}
          </button>
        ))}
      </div>
      {children}
    </section>
  );
}

/** A grid that draws a long list a part at a time, adding the next part when its end scrolls into view. */
function GrowingGrid({
  label,
  choices,
  ...handlers
}: CellHandlers & { label: string; choices: EmojiChoice[] }) {
  const [shown, setShown] = useState(CHUNK);
  const sentinel = useRef<HTMLDivElement>(null);
  const more = shown < choices.length;
  useEffect(() => {
    const element = sentinel.current;
    if (!element || !more) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) setShown((count) => count + CHUNK);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [more]);
  return (
    <Grid label={label} choices={choices.slice(0, shown)} first {...handlers}>
      {more && <div ref={sentinel} className="h-8" />}
    </Grid>
  );
}
