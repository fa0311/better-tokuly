import { TextField } from "#/components/ui/field";
import { useTranslation } from "#/lib/use-translation";

/** What people can say about a stamp, as typed in the form. */
export interface StampMeta {
  name: string;
  emoji: string;
  /** Search words as free text: separated by spaces or commas. */
  keywords: string;
}

export const EMPTY_META: StampMeta = { name: "", emoji: "", keywords: "" };

export const MAX_NAME_LENGTH = 40;
const MAX_KEYWORDS = 10;
const MAX_KEYWORD_LENGTH = 24;

/**
 * Search words from free text: split on spaces and commas (also the
 * Japanese ones), without '#', lowercase, no repeats, within the API's
 * limits (10 words of at most 24 characters).
 */
export function parseKeywords(text: string): string[] {
  const words = text
    .split(/[\s,、，]+/u)
    .map((word) =>
      Array.from(word.replace(/^#+/, "").toLowerCase()).slice(0, MAX_KEYWORD_LENGTH).join(""),
    )
    .filter(Boolean);
  return [...new Set(words)].slice(0, MAX_KEYWORDS);
}

/** A file name turned into a starting name for a stamp: no extension, no separators. */
export function nameFromFile(fileName: string): string {
  const base = fileName
    .replace(/\.[^.]+$/, "")
    .replace(/[_-]+/g, " ")
    .trim();
  return Array.from(base).slice(0, MAX_NAME_LENGTH).join("");
}

/** The name, emoji and search-word fields of a stamp. */
export function StampMetaFields({
  value,
  onChange,
  autoFocus,
}: {
  value: StampMeta;
  onChange(value: StampMeta): void;
  autoFocus?: boolean;
}) {
  const { t } = useTranslation("stamps");
  return (
    <div className="flex flex-col gap-3">
      <TextField
        label={t("meta.name")}
        placeholder={t("meta.namePlaceholder")}
        value={value.name}
        maxLength={MAX_NAME_LENGTH}
        autoFocus={autoFocus}
        onChange={(event) => onChange({ ...value, name: event.target.value })}
      />
      <div className="grid grid-cols-[6.5rem_1fr] gap-3">
        <TextField
          label={t("meta.emoji")}
          placeholder="😆"
          value={value.emoji}
          maxLength={16}
          onChange={(event) => onChange({ ...value, emoji: event.target.value.trim() })}
        />
        <TextField
          label={t("meta.keywords")}
          placeholder={t("meta.keywordsPlaceholder")}
          value={value.keywords}
          maxLength={200}
          onChange={(event) => onChange({ ...value, keywords: event.target.value })}
        />
      </div>
      <p className="px-1 text-xs text-ink-faint">{t("meta.hint")}</p>
    </div>
  );
}
