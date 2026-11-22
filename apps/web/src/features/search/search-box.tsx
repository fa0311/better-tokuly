import type { search } from "@tokuly/api";
import { Search, X } from "lucide-react";
import { type FormEvent, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { MAX_QUERY_LENGTH } from "./params";
import { optionId, SuggestionList, useOpenSuggestion, useSuggestionKeys } from "./suggestion-list";
import { useSuggestions } from "./typeahead";

/** Marks the page's own search field, so the "/" shortcut can focus it instead of opening the palette. */
export const SEARCH_BOX_ATTRIBUTE = "data-search-box";

/**
 * The search field of `/search`: answers while typing with completions and
 * things to jump to; arrow keys walk the list, Enter searches for the text or
 * opens the selected line, Escape closes the list.
 */
export function SearchBox({
  query,
  kind,
  onSearch,
  autoFocus,
  className,
}: {
  /** The query the page shows (from the address); the field follows it. */
  query: string;
  /** The open tab: typeahead hits are limited to it. */
  kind?: search.SearchKind;
  onSearch(query: string): void;
  autoFocus?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("search");
  const listId = useId();
  const input = useRef<HTMLInputElement>(null);
  const [text, setText] = useState(query);
  const [open, setOpen] = useState(false);
  // Back, forward and links change the query from outside: the field shows it.
  const [applied, setApplied] = useState(query);
  if (applied !== query) {
    setApplied(query);
    setText(query);
  }

  // Before a first search the page under the field already shows recent searches and trends.
  const { suggestions, loading } = useSuggestions(text, { kind, idle: query !== "", enabled: open });
  const submit = (value: string) => {
    const next = value.trim();
    setOpen(false);
    if (!next) return;
    setText(next);
    input.current?.blur();
    onSearch(next);
  };
  const openSuggestion = useOpenSuggestion(submit);
  const keys = useSuggestionKeys({
    suggestions: open ? suggestions : [],
    onChoose: (suggestion) => {
      setOpen(false);
      input.current?.blur();
      openSuggestion(suggestion);
    },
    onSubmit: () => submit(text),
    onEscape: () => setOpen(false),
  });
  const expanded = open && suggestions.length > 0;

  return (
    <search className={cn("relative block", className)}>
      <form
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          submit(text);
        }}
        className="sticker flex h-14 items-center gap-2 rounded-full bg-surface pr-2 pl-5 focus-within:outline-[3px] focus-within:outline-offset-2 focus-within:outline-brand focus-within:outline-solid"
      >
        <Search className="size-5 shrink-0 text-ink-soft" aria-hidden />
        <input
          ref={input}
          {...{ [SEARCH_BOX_ATTRIBUTE]: "" }}
          role="combobox"
          aria-label={t("box.label")}
          aria-expanded={expanded}
          aria-controls={expanded ? listId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={expanded && keys.active >= 0 ? optionId(listId, keys.active) : undefined}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={keys.onKeyDown}
          placeholder={t("box.placeholder")}
          maxLength={MAX_QUERY_LENGTH}
          // biome-ignore lint/a11y/noAutofocus: the explore page exists to be typed into.
          autoFocus={autoFocus}
          enterKeyHint="search"
          autoCapitalize="none"
          autoComplete="off"
          autoCorrect="off"
          spellCheck={false}
          className="h-full min-w-0 flex-1 bg-transparent text-[17px] font-bold outline-none placeholder:font-medium placeholder:text-ink-faint"
        />
        {loading && open && <Spinner className="size-5 shrink-0 text-ink-faint" />}
        {text && (
          <button
            type="button"
            aria-label={t("box.clear")}
            // Keep the field focused: clearing is the start of typing something else.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              setText("");
              setOpen(true);
              input.current?.focus();
            }}
            className="grid size-10 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
          >
            <X className="size-5" />
          </button>
        )}
      </form>
      {expanded && (
        <div className="sticker absolute inset-x-0 top-full z-30 mt-2 max-h-[min(26rem,60dvh)] overflow-y-auto rounded-3xl bg-surface p-1.5">
          <SuggestionList listId={listId} label={t("suggest.label")} suggestions={suggestions} active={keys.active} onActive={keys.setActive} onChoose={(suggestion) => {
            setOpen(false);
            input.current?.blur();
            openSuggestion(suggestion);
          }} />
        </div>
      )}
    </search>
  );
}
