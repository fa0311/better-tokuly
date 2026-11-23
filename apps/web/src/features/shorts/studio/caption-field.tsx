import { useQuery } from "@tanstack/react-query";
import { identity } from "@tokuly/api";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { TextArea } from "#/components/ui/field";
import { EmojiSuggestBar } from "#/features/emoji";
import { compactCount } from "#/features/engagement";
import { api } from "#/lib/api";
import { discoveryClient, shortKeys } from "../api";
import { completeToken, MAX_CAPTION, tokenAtCaret } from "../model";

const chip =
  "flex h-9 shrink-0 items-center gap-1.5 rounded-full border-2 border-line bg-surface px-3 text-sm font-bold hover:border-ink-faint";

/**
 * The caption of a short: text with `#hashtags` and `@mentions`, both
 * suggested while they are typed (the hashtags people use most, the people
 * the name matches), and `:emoji:` completion.
 */
export function CaptionField({
  value,
  onChange,
}: {
  value: string;
  onChange(value: string): void;
}) {
  const { t, i18n } = useTranslation("shorts");
  const field = useRef<HTMLTextAreaElement | null>(null);
  const [caret, setCaret] = useState(0);
  const token = tokenAtCaret(value, caret);
  const hashtagPrefix = token?.kind === "hashtag" ? token.query : "";
  const mentionQuery = token?.kind === "mention" ? token.query : "";

  const hashtags = useQuery({
    queryKey: shortKeys.hashtagSuggestions(hashtagPrefix),
    queryFn: async () =>
      (await discoveryClient().suggestHashtags({ prefix: hashtagPrefix, limit: 8 })).hashtags,
    enabled: hashtagPrefix.length > 0,
    staleTime: 60_000,
  });
  const people = useQuery({
    queryKey: ["identity", "search", "mention", mentionQuery],
    queryFn: async () =>
      (
        await api()
          .service(identity.ProfileService)
          .searchUsers({ query: mentionQuery, page: { limit: 6, cursor: "" } })
      ).results.flatMap((result) => (result.user ? [result.user] : [])),
    enabled: mentionQuery.length > 0,
    staleTime: 60_000,
  });

  const put = (next: { text: string; caret: number }) => {
    onChange(next.text.slice(0, MAX_CAPTION));
    setCaret(next.caret);
    // After the field has the new text, the caret goes behind what was inserted.
    requestAnimationFrame(() => {
      field.current?.focus();
      field.current?.setSelectionRange(next.caret, next.caret);
    });
  };
  const complete = (replacement: string) => {
    if (token) put(completeToken(value, token.start, caret, replacement));
  };
  const track = (element: HTMLTextAreaElement) => {
    field.current = element;
    setCaret(element.selectionStart);
  };

  const tagChoices = hashtagPrefix ? (hashtags.data ?? []) : [];
  const peopleChoices = mentionQuery ? (people.data ?? []) : [];

  return (
    <div className="flex flex-col gap-2">
      <TextArea
        label={t("form.caption")}
        value={value}
        maxLength={MAX_CAPTION}
        rows={4}
        placeholder={t("form.captionPlaceholder")}
        hint={t("form.captionHint", { count: value.length, max: MAX_CAPTION })}
        onChange={(event) => {
          onChange(event.target.value);
          track(event.target);
        }}
        onSelect={(event) => track(event.currentTarget)}
      />
      {(tagChoices.length > 0 || peopleChoices.length > 0) && (
        <div
          className="scrollbar-none -mx-1 flex gap-1.5 overflow-x-auto px-1"
          role="listbox"
          aria-label={t("form.suggestions")}
        >
          {tagChoices.map((hashtag) => (
            <button
              key={hashtag.tag}
              type="button"
              role="option"
              aria-selected={false}
              className={chip}
              // The field keeps the focus (and the keyboard stays up).
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => complete(`#${hashtag.tag}`)}
            >
              #{hashtag.tag}
              <span className="text-xs text-ink-faint">
                {compactCount(Number(hashtag.shortCount), i18n.language)}
              </span>
            </button>
          ))}
          {peopleChoices.map((user) => (
            <button
              key={user.id}
              type="button"
              role="option"
              aria-selected={false}
              className={chip}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => complete(`@${user.handle}`)}
            >
              <Avatar user={user} size="xs" />@{user.handle}
            </button>
          ))}
        </div>
      )}
      <EmojiSuggestBar text={value} caret={caret} onComplete={put} />
    </div>
  );
}
