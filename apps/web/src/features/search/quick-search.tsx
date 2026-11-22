import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowDown, ArrowUp, CornerDownLeft, Search, X } from "lucide-react";
import { type ReactNode, type RefObject, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { submitSearch } from "./api";
import { useHasKeyboard } from "./hooks";
import { MAX_QUERY_LENGTH } from "./params";
import { optionId, SuggestionList, useOpenSuggestion, useSuggestionKeys } from "./suggestion-list";
import { useSuggestions } from "./typeahead";

const usePalette = create<{ open: boolean; query: string }>(() => ({ open: false, query: "" }));

/** Opens the quick search over whatever page is showing, optionally with something typed already. */
export function openQuickSearch(query = ""): void {
  usePalette.setState({ open: true, query });
}

export function closeQuickSearch(): void {
  usePalette.setState({ open: false });
}

export function toggleQuickSearch(): void {
  usePalette.setState((state) => ({ open: !state.open, query: "" }));
}

/**
 * Quick search: a command palette that answers while typing and jumps
 * straight to a person, a tag or a result page. Mounted once for the whole
 * app (`background.tsx`); open it with Cmd/Ctrl+K, "/" or `openQuickSearch()`.
 */
export function QuickSearch() {
  const { t } = useTranslation("search");
  const open = usePalette((state) => state.open);
  const field = useRef<HTMLInputElement>(null);
  return (
    <BaseDialog.Root open={open} onOpenChange={(next) => usePalette.setState({ open: next })}>
      <BaseDialog.Portal>
        <BaseDialog.Backdrop className="fixed inset-0 z-50 bg-overlay transition-opacity duration-150 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0" />
        <BaseDialog.Popup
          initialFocus={field}
          aria-label={t("palette.title")}
          data-quick-search=""
          className={cn(
            "sticker-lg fixed inset-x-0 top-0 z-50 flex max-h-[85dvh] flex-col rounded-b-[2rem] border-t-0 bg-surface outline-none",
            "transition-all duration-200 ease-out-soft data-[ending-style]:-translate-y-4 data-[ending-style]:opacity-0 data-[starting-style]:-translate-y-4 data-[starting-style]:opacity-0",
            "sm:inset-x-auto sm:top-[12vh] sm:left-1/2 sm:max-h-[70dvh] sm:w-[min(40rem,calc(100vw-2rem))] sm:-translate-x-1/2 sm:rounded-[2rem] sm:border-t-[3px]",
          )}
        >
          <Palette field={field} />
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

/** The inside of the palette; mounted while it is open, so each opening starts fresh. */
function Palette({ field }: { field: RefObject<HTMLInputElement | null> }) {
  const { t } = useTranslation("search");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const keyboard = useHasKeyboard();
  const listId = useId();
  const [text, setText] = useState(() => usePalette.getState().query);
  const { suggestions, loading } = useSuggestions(text);

  const runSearch = (query: string) => {
    const next = query.trim();
    if (!next) return;
    submitSearch(queryClient, next);
    closeQuickSearch();
    void navigate({ to: "/search", search: { q: next } });
  };
  const openSuggestion = useOpenSuggestion(runSearch);
  const choose: typeof openSuggestion = (suggestion) => {
    closeQuickSearch();
    openSuggestion(suggestion);
  };
  const keys = useSuggestionKeys({
    suggestions,
    onChoose: choose,
    onSubmit: () => runSearch(text),
  });

  return (
    <>
      <div className="flex h-16 shrink-0 items-center gap-3 border-b-2 border-line pr-3 pl-5 pt-safe sm:pt-0">
        <Search className="size-5 shrink-0 text-ink-soft" aria-hidden />
        <input
          ref={field}
          role="combobox"
          aria-label={t("palette.title")}
          aria-expanded={suggestions.length > 0}
          aria-controls={suggestions.length > 0 ? listId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={keys.active >= 0 ? optionId(listId, keys.active) : undefined}
          value={text}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={keys.onKeyDown}
          placeholder={t("palette.placeholder")}
          maxLength={MAX_QUERY_LENGTH}
          enterKeyHint="search"
          autoCapitalize="none"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          className="h-full min-w-0 flex-1 bg-transparent text-[17px] font-bold outline-none placeholder:font-medium placeholder:text-ink-faint"
        />
        {loading && <Spinner className="size-5 shrink-0 text-ink-faint" />}
        <BaseDialog.Close
          aria-label={t("palette.close")}
          className="grid h-9 min-w-9 shrink-0 place-items-center rounded-full px-2 text-ink-soft hover:bg-surface-2"
        >
          {keyboard ? <Keycap>esc</Keycap> : <X className="size-5" />}
        </BaseDialog.Close>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
        {suggestions.length > 0 ? (
          <SuggestionList
            listId={listId}
            label={t("suggest.label")}
            suggestions={suggestions}
            active={keys.active}
            onActive={keys.setActive}
            onChoose={choose}
          />
        ) : (
          <p className="px-4 py-8 text-center text-sm leading-relaxed text-ink-soft">
            {t("palette.hint")}
          </p>
        )}
      </div>
      <footer className="flex shrink-0 items-center gap-4 border-t-2 border-line px-5 py-2.5 text-xs font-bold text-ink-faint pb-safe sm:pb-2.5">
        {keyboard && (
          <>
            <span className="inline-flex items-center gap-1">
              <Keycap>
                <ArrowUp className="size-3" />
              </Keycap>
              <Keycap>
                <ArrowDown className="size-3" />
              </Keycap>
              {t("palette.keys.move")}
            </span>
            <span className="inline-flex items-center gap-1">
              <Keycap>
                <CornerDownLeft className="size-3" />
              </Keycap>
              {t("palette.keys.open")}
            </span>
          </>
        )}
        <Link
          to="/search"
          onClick={closeQuickSearch}
          className="ml-auto text-brand hover:underline"
        >
          {t("palette.openPage")}
        </Link>
      </footer>
    </>
  );
}

function Keycap({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-grid h-5 min-w-5 place-items-center rounded-md border-2 border-line bg-surface-2 px-1 font-sans text-[10px] font-black text-ink-soft">
      {children}
    </kbd>
  );
}
