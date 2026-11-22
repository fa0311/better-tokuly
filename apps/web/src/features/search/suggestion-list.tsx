import { useRouter } from "@tanstack/react-router";
import { search } from "@tokuly/api";
import { BadgeCheck, Clock, CornerDownLeft, Search, TrendingUp, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { cn } from "#/lib/cn";
import { useHistoryActions } from "./api";
import { useHasKeyboard } from "./hooks";
import { KindTile } from "./kind-icon";
import { hitHref } from "./hit-href";
import { extraFlag, extraNumber, extraText, hitExtra, isKind, kindSlug } from "./kinds";
import type { Suggestion, SuggestionGroup } from "./suggestions";

/**
 * Arrow keys, Enter and Escape for a search field with a suggestion list.
 * Nothing is selected until an arrow key is pressed, so Enter searches for
 * what was typed. Keys pressed while an input method is composing (choosing
 * kanji) belong to it, not to the list.
 */
export function useSuggestionKeys({ suggestions, onChoose, onSubmit, onEscape }: { suggestions: Suggestion[]; onChoose(suggestion: Suggestion): void; onSubmit(): void; onEscape?(): void }) {
  const [active, setActive] = useState(-1);
  // A different list starts without a selection.
  const signature = suggestions.map((suggestion) => suggestion.id).join("\n");
  const [shown, setShown] = useState(signature);
  if (shown !== signature) {
    setShown(signature);
    setActive(-1);
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    const last = suggestions.length - 1;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive(active >= last ? (last >= 0 ? 0 : -1) : active + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive(active < 0 ? last : active - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const chosen = suggestions[active];
      if (chosen) onChoose(chosen);
      else onSubmit();
    } else if (event.key === "Escape" && onEscape) {
      event.preventDefault();
      onEscape();
    }
  };
  return { active, setActive, onKeyDown };
}

/** Opens what a suggestion stands for: a result page for a query, the subject's own page for a hit. */
export function useOpenSuggestion(onSearch: (query: string) => void): (suggestion: Suggestion) => void {
  const router = useRouter();
  return (suggestion) => {
    if (suggestion.type === "hit") void router.navigate({ href: hitHref(suggestion.hit) });
    else onSearch(suggestion.query);
  };
}

export const optionId = (listId: string, index: number) => `${listId}-option-${index}`;

/**
 * The suggestions of a search field as a listbox. The field keeps the focus
 * (`aria-activedescendant` points at the active option), so typing never
 * stops.
 */
export function SuggestionList({
  listId,
  label,
  suggestions,
  active,
  onActive,
  onChoose,
  className,
}: {
  listId: string;
  label: string;
  suggestions: Suggestion[];
  active: number;
  onActive(index: number): void;
  onChoose(suggestion: Suggestion): void;
  className?: string;
}) {
  const { t } = useTranslation("search");
  const { forget } = useHistoryActions();

  useEffect(() => {
    if (active >= 0) document.getElementById(optionId(listId, active))?.scrollIntoView({ block: "nearest" });
  }, [active, listId]);

  const headings: Partial<Record<SuggestionGroup, string>> = { recent: t("recent.title"), trending: t("trending.title"), hits: t("suggest.jumpTo") };
  return (
    // Clicking the list must not take the focus away from the field (which would close it before the click lands).
    <div id={listId} role="listbox" aria-label={label} className={cn("flex flex-col", className)} onMouseDown={(event) => event.preventDefault()}>
      {suggestions.map((suggestion, index) => {
        const heading = suggestion.group !== suggestions[index - 1]?.group ? headings[suggestion.group] : undefined;
        const selected = index === active;
        return (
          <div key={suggestion.id} role="presentation">
            {heading && <p className="px-3 pt-3 pb-1 text-xs font-black text-ink-faint">{heading}</p>}
            <div className={cn("flex items-center rounded-2xl", selected && "bg-surface-2")}>
              {/* biome-ignore lint/a11y/useFocusableInteractive: the focus stays in the field, which names the active option with aria-activedescendant and handles the keys. */}
              <div
                id={optionId(listId, index)}
                role="option"
                aria-selected={selected}
                onClick={() => onChoose(suggestion)}
                onPointerMove={() => !selected && onActive(index)}
                className="flex min-h-12 min-w-0 flex-1 cursor-pointer items-center gap-3 px-3 py-1.5"
              >
                <SuggestionFace suggestion={suggestion} selected={selected} />
              </div>
              {suggestion.type === "query" && suggestion.recentId && (
                <button
                  type="button"
                  tabIndex={-1}
                  aria-label={t("recent.forget", { query: suggestion.query })}
                  onClick={() => forget.mutate(suggestion.recentId as string)}
                  className="mr-1.5 grid size-9 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-3 hover:text-ink"
                >
                  <X className="size-4" />
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

const iconTile = "grid size-9 shrink-0 place-items-center rounded-xl bg-surface-2 text-ink-soft";

function SuggestionFace({ suggestion, selected }: { suggestion: Suggestion; selected: boolean }) {
  const { t } = useTranslation("search");
  const keyboard = useHasKeyboard();
  if (suggestion.type === "search") {
    return (
      <>
        <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-brand text-on-brand">
          <Search className="size-[18px]" />
        </span>
        <span className="min-w-0 flex-1 truncate font-bold">{t("suggest.searchFor", { query: suggestion.query })}</span>
        {keyboard && selected && <CornerDownLeft className="size-4 shrink-0 text-ink-faint" aria-hidden />}
      </>
    );
  }
  if (suggestion.type === "query") {
    return (
      <>
        {suggestion.source === "trending" ? (
          <span className={cn("grid size-9 shrink-0 place-items-center rounded-xl text-sm font-black tabular-nums", (suggestion.rank ?? 9) <= 3 ? "bg-pink text-white" : "bg-surface-2 text-ink-soft")}>{suggestion.rank}</span>
        ) : (
          <span className={iconTile}>{suggestion.source === "recent" ? <Clock className="size-[18px]" /> : <TrendingUp className="size-[18px]" />}</span>
        )}
        <span className="min-w-0 flex-1 truncate font-bold">{suggestion.query}</span>
        <span className="sr-only">{t(`suggest.source.${suggestion.source}`)}</span>
      </>
    );
  }
  return <HitFace hit={suggestion.hit} />;
}

/** A top hit in one line: its picture (or its kind's icon), its name and what it is. */
function HitFace({ hit }: { hit: search.SearchHit }) {
  const { t } = useTranslation("search");
  const extra = hitExtra(hit);
  const kindName = isKind(hit.kind) ? t(`kinds.${kindSlug(hit.kind)}`) : "";
  let detail = kindName;
  if (hit.kind === search.SearchKind.USER || hit.kind === search.SearchKind.CHANNEL) {
    const handle = extraText(extra, "handle") || (hit.kind === search.SearchKind.USER ? hit.owner?.handle : "");
    detail = handle ? `@${handle} · ${kindName}` : kindName;
  } else if (hit.kind === search.SearchKind.HASHTAG) {
    const count = extraNumber(extra, "note_count");
    detail = count === undefined ? kindName : t("hit.noteCount", { count });
  } else if (hit.owner && !hit.owner.deleted) {
    detail = `${kindName} · ${hit.owner.displayName || hit.owner.handle}`;
  }
  return (
    <>
      {hit.kind === search.SearchKind.USER && hit.owner ? (
        <Avatar user={hit.owner} size="sm" className="mx-0.5" />
      ) : hit.imageUrl ? (
        <img src={hit.imageUrl} alt="" className="size-9 shrink-0 rounded-xl bg-surface-3 object-cover" loading="lazy" decoding="async" draggable={false} />
      ) : (
        <KindTile kind={hit.kind} className="size-9 rounded-xl" iconClassName="size-[18px]" />
      )}
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1">
          <span className="truncate font-bold">{hit.title}</span>
          {extraFlag(extra, "verified") && <BadgeCheck className="size-4 shrink-0 text-brand" aria-label={t("hit.verified")} />}
        </span>
        <span className="block truncate text-xs text-ink-soft">{detail}</span>
      </span>
    </>
  );
}
