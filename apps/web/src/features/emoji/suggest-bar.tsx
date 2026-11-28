import { shortcodePrefixAt } from "@tokuly/core/emoji";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { EmojiImage } from "./emoji";
import { useEmojiSuggest } from "./suggest";
import { recordEmojiUse } from "./use";

/** The text with the `:prefix` between `start` and `caret` replaced by an emoji, and where the caret goes. */
export function completeShortcode(
  text: string,
  start: number,
  caret: number,
  emoji: string,
): { text: string; caret: number } {
  const before = text.slice(0, start);
  const after = text.slice(caret);
  const inserted = `${emoji}${after.startsWith(" ") ? "" : " "}`;
  return { text: before + inserted + after, caret: before.length + inserted.length };
}

export interface EmojiSuggestBarProps {
  /** The composer's text and where its caret is. */
  text: string;
  caret: number;
  /** Called with the text after the pick and where the caret belongs; put both into the field. */
  onComplete(next: { text: string; caret: number }): void;
  className?: string;
}

/**
 * `:` autocomplete for a composer: while a `:name` is being typed at the
 * caret, a row of matching emoji (custom and Unicode) to tap. Renders
 * nothing otherwise, so it can sit right above or below the text field:
 *
 *   <EmojiSuggestBar text={text} caret={caret} onComplete={({ text, caret }) => { setText(text); moveCaret(caret); }} />
 *
 * The field keeps the focus while a suggestion is pressed.
 */
export function EmojiSuggestBar({ text, caret, onComplete, className }: EmojiSuggestBarProps) {
  const { t } = useTranslation("emoji");
  const typing = shortcodePrefixAt(text, caret);
  const suggestions = useEmojiSuggest(typing?.prefix ?? "");
  if (!typing || suggestions.length === 0) return null;
  return (
    <ul
      aria-label={t("detail.suggestions")}
      className={cn("scrollbar-none flex gap-1.5 overflow-x-auto pb-1", className)}
    >
      {suggestions.map((suggestion) => (
        <li key={suggestion.emoji} className="shrink-0">
          <button
            type="button"
            // A press must not take the focus (and the keyboard) away from the field.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              recordEmojiUse([suggestion.emoji]);
              onComplete(completeShortcode(text, typing.start, caret, suggestion.emoji));
            }}
            className="flex h-9 items-center gap-1.5 rounded-full border-2 border-line bg-surface px-3 text-sm font-bold text-ink hover:border-ink-faint"
          >
            {suggestion.kind === "custom" ? (
              <EmojiImage emoji={suggestion.custom} size={20} className="size-5" />
            ) : (
              <span aria-hidden>{suggestion.emoji}</span>
            )}
            <span>{suggestion.kind === "custom" ? suggestion.emoji : suggestion.name}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
