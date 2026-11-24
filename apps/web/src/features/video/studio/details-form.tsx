import { useQuery } from "@tanstack/react-query";
import { common, video } from "@tokuly/api";
import { Clock, Plus, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { videoClient, videoKeys } from "../api";
import { CircleField, LockedNote } from "../audience";
import { CATEGORIES, VIDEO_VISIBILITIES } from "../labels";
import {
  type ChapterDraft,
  MAX_CHAPTERS,
  normalizeTag,
  toDrafts,
  validateChapters,
  withChapterAt,
} from "./chapters";
import { FormSection } from "./shared";

const MAX_TAGS = 20;
const NO_CATEGORY = "0";

/** Everything the details tab edits, as the form holds it. */
export interface DetailsDraft {
  title: string;
  description: string;
  tags: string[];
  category: video.VideoCategory;
  game: string;
  chapters: ChapterDraft[];
  visibility: common.Visibility;
  /** For CIRCLE: one of the caller's circles. */
  circleId: string;
  allowLink: boolean;
  ageRestricted: boolean;
  commentsEnabled: boolean;
  notifySubscribers: boolean;
}

export function draftFrom(item: video.Video): DetailsDraft {
  return {
    title: item.summary?.title ?? "",
    description: item.description,
    tags: [...item.tags],
    category: item.summary?.category ?? video.VideoCategory.UNSPECIFIED,
    game: item.game,
    chapters: toDrafts(item.chapters),
    visibility: item.summary?.visibility || common.Visibility.PRIVATE,
    circleId: item.studio?.circleId ?? "",
    allowLink: item.allowLink,
    ageRestricted: item.summary?.ageRestricted ?? false,
    commentsEnabled: item.commentsEnabled,
    notifySubscribers: item.studio?.notifySubscribers ?? true,
  };
}

/** The draft without what only the editor needs (row keys), to tell whether anything was changed. */
export function draftSignature(draft: DetailsDraft): string {
  return JSON.stringify({
    ...draft,
    chapters: draft.chapters.map((chapter) => [chapter.time, chapter.title]),
  });
}

/** Whether the draft can be saved: it has a title and its chapters are in order. */
export function draftProblem(
  draft: DetailsDraft,
  durationSeconds: number,
): "title" | "chapters" | null {
  if (!draft.title.trim()) return "title";
  return validateChapters(draft.chapters, durationSeconds).some(Boolean) ? "chapters" : null;
}

/**
 * The details of a video: title and description, tags, genre and game,
 * chapters, who may see it, and the switches (age restriction, comments,
 * telling subscribers).
 */
export function DetailsForm({
  draft,
  onChange,
  durationSeconds,
  playerTime,
}: {
  draft: DetailsDraft;
  onChange(draft: DetailsDraft): void;
  durationSeconds: number /** The preview player's position, for "add a chapter here"; undefined while there is no player. */;
  playerTime?: () => number;
}) {
  const { t } = useTranslation("video");
  const set = <K extends keyof DetailsDraft>(key: K, value: DetailsDraft[K]) =>
    onChange({ ...draft, [key]: value });

  return (
    <div className="flex flex-col gap-5">
      <FormSection title={t("studio.editor.basics")}>
        <TextField
          label={t("studio.editor.title")}
          value={draft.title}
          maxLength={100}
          placeholder={t("studio.editor.titlePlaceholder")}
          error={draft.title.trim() ? undefined : t("studio.editor.titleRequired")}
          onChange={(event) => set("title", event.target.value)}
        />
        <TextArea
          label={t("studio.editor.description")}
          value={draft.description}
          maxLength={5000}
          placeholder={t("studio.editor.descriptionPlaceholder")}
          hint={t("studio.editor.descriptionHint")}
          onChange={(event) => set("description", event.target.value)}
        />
        <TagsField tags={draft.tags} onChange={(tags) => set("tags", tags)} />
        <div className="grid gap-4 sm:grid-cols-2">
          <SelectField
            label={t("studio.editor.category")}
            value={String(draft.category)}
            onChange={(value) => set("category", Number(value) as video.VideoCategory)}
            options={[
              { value: NO_CATEGORY, label: t("studio.editor.noCategory") },
              ...CATEGORIES.map((entry) => ({
                value: String(entry.value),
                label: `${entry.emoji} ${t(`category.${entry.key}`)}`,
              })),
            ]}
          />
          <GameField game={draft.game} onChange={(game) => set("game", game)} />
        </div>
      </FormSection>

      <FormSection title={t("studio.chapters.title")} lead={t("studio.chapters.lead")}>
        <ChaptersField
          chapters={draft.chapters}
          durationSeconds={durationSeconds}
          playerTime={playerTime}
          onChange={(chapters) => set("chapters", chapters)}
        />
      </FormSection>

      <FormSection title={t("studio.editor.audience")} lead={t("studio.editor.audienceLead")}>
        <div
          role="radiogroup"
          aria-label={t("studio.editor.audience")}
          className="flex flex-col gap-2"
        >
          {VIDEO_VISIBILITIES.map((option) => {
            const Icon = option.icon;
            const selected = draft.visibility === option.value;
            return (
              // biome-ignore lint/a11y/useSemanticElements: a card-sized choice with an icon and an explanation; a native radio cannot carry this layout.
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={selected}
                className={cn(
                  "flex items-center gap-3 rounded-2xl border-2 p-3 text-left transition-colors",
                  selected
                    ? "border-ink bg-brand-soft"
                    : "border-line bg-surface hover:border-ink-faint",
                )}
                onClick={() => set("visibility", option.value)}
              >
                <span
                  className={cn(
                    "grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink",
                    selected ? "bg-brand text-on-brand" : "bg-surface-2",
                  )}
                >
                  <Icon className="size-5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block font-black">{t(`visibility.${option.key}`)}</span>
                  <span className="block text-sm text-ink-soft">
                    {t(`visibility.${option.key}Hint`)}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
        {draft.visibility === common.Visibility.CIRCLE && (
          <CircleField
            label={t("visibility.circlePick")}
            emptyText={t("visibility.circleNone")}
            value={draft.circleId}
            onChange={(circleId) => set("circleId", circleId)}
          />
        )}
        {draft.visibility === common.Visibility.FRIENDS && (
          <SwitchRow
            label={t("studio.editor.allowLink")}
            hint={t("studio.editor.allowLinkHint")}
            checked={draft.allowLink}
            onChange={(value) => set("allowLink", value)}
          />
        )}
        <LockedNote text={t("visibility.lockedNote")} />
      </FormSection>

      <FormSection title={t("studio.editor.options")}>
        <div className="-mx-2 -my-2">
          <SwitchRow
            label={t("studio.editor.comments")}
            hint={t("studio.editor.commentsHint")}
            checked={draft.commentsEnabled}
            onChange={(value) => set("commentsEnabled", value)}
          />
          <SwitchRow
            label={t("studio.editor.notify")}
            hint={t("studio.editor.notifyHint")}
            checked={draft.notifySubscribers}
            onChange={(value) => set("notifySubscribers", value)}
          />
          <SwitchRow
            label={t("studio.editor.ageRestricted")}
            hint={t("studio.editor.ageRestrictedHint")}
            checked={draft.ageRestricted}
            onChange={(value) => set("ageRestricted", value)}
          />
        </div>
      </FormSection>
    </div>
  );
}

/** Tags as chips: Enter, a comma or a space adds what was typed; Backspace on an empty field takes the last one off. */
function TagsField({ tags, onChange }: { tags: string[]; onChange(tags: string[]): void }) {
  const { t } = useTranslation("video");
  const id = useId();
  const [text, setText] = useState("");
  const add = (input: string) => {
    const next = input
      .split(/[,、\s]+/)
      .map(normalizeTag)
      .filter((tag) => tag && !tags.includes(tag));
    setText("");
    if (next.length > 0) onChange([...tags, ...new Set(next)].slice(0, MAX_TAGS));
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // While an input method is composing (Japanese), Enter confirms the conversion, not the tag.
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Enter" || event.key === ",") {
      event.preventDefault();
      add(text);
    } else if (event.key === "Backspace" && !text && tags.length > 0) {
      onChange(tags.slice(0, -1));
    }
  };
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="px-1 text-sm font-bold text-ink-soft">
        {t("studio.editor.tags")}
      </label>
      <div className="flex min-h-12 flex-wrap items-center gap-1.5 rounded-control border-2 border-line bg-surface px-2 py-1.5 focus-within:border-brand">
        {tags.map((tag) => (
          <span
            key={tag}
            className="inline-flex h-8 items-center gap-0.5 rounded-full bg-surface-2 pr-1 pl-3 text-sm font-bold"
          >
            #{tag}
            <button
              type="button"
              aria-label={t("studio.editor.removeTag", { tag })}
              className="grid size-6 place-items-center rounded-full hover:bg-surface-3"
              onClick={() => onChange(tags.filter((existing) => existing !== tag))}
            >
              <X className="size-3.5" />
            </button>
          </span>
        ))}
        <input
          id={id}
          value={text}
          disabled={tags.length >= MAX_TAGS}
          maxLength={60}
          placeholder={tags.length === 0 ? t("studio.editor.tagsPlaceholder") : ""}
          className="h-8 min-w-24 flex-1 bg-transparent px-2 text-[15px] outline-none placeholder:text-ink-faint"
          onChange={(event) => setText(event.target.value)}
          onKeyDown={onKeyDown}
          onBlur={() => text.trim() && add(text)}
        />
      </div>
      <p className="px-1 text-xs text-ink-faint">
        {t("studio.editor.tagsHint", { count: tags.length, max: MAX_TAGS })}
      </p>
    </div>
  );
}

/** The game being played, with titles other creators used offered while typing. */
function GameField({ game, onChange }: { game: string; onChange(game: string): void }) {
  const { t } = useTranslation("video");
  const listId = useId();
  const [query, setQuery] = useState(game);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(game.trim()), 250);
    return () => clearTimeout(timer);
  }, [game]);
  const suggestions = useQuery({
    queryKey: videoKeys.games(query),
    queryFn: () => videoClient().suggestGames({ query, limit: 8 }),
    staleTime: 60_000,
  });
  return (
    <>
      <TextField
        label={t("studio.editor.game")}
        value={game}
        maxLength={100}
        list={listId}
        autoComplete="off"
        placeholder={t("studio.editor.gamePlaceholder")}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={listId}>
        {suggestions.data?.games.map((title) => (
          <option key={title} value={title} />
        ))}
      </datalist>
    </>
  );
}

/** Chapters as rows of a start time and a name; each row says what is wrong with it, if anything. */
function ChaptersField({
  chapters,
  durationSeconds,
  playerTime,
  onChange,
}: {
  chapters: ChapterDraft[];
  durationSeconds: number;
  playerTime?: () => number;
  onChange(chapters: ChapterDraft[]): void;
}) {
  const { t } = useTranslation("video");
  const errors = validateChapters(chapters, durationSeconds);
  const patch = (key: string, change: Partial<ChapterDraft>) =>
    onChange(
      chapters.map((chapter) => (chapter.key === key ? { ...chapter, ...change } : chapter)),
    );
  // Without a player the next row starts ten seconds after the last one.
  const lastStart = chapters.length;
  const addAt = () => onChange(withChapterAt(chapters, playerTime ? playerTime() : lastStart * 10));

  return (
    <>
      {chapters.length === 0 && (
        <p className="text-sm text-ink-faint">{t("studio.chapters.empty")}</p>
      )}
      <ol className="flex flex-col gap-2">
        {chapters.map((chapter, index) => {
          const error = errors[index];
          return (
            <li key={chapter.key} data-testid="chapter-row">
              <div className="flex items-center gap-2">
                <input
                  value={chapter.time}
                  aria-label={t("studio.chapters.time", { index: index + 1 })}
                  aria-invalid={
                    error === "time" ||
                    error === "first" ||
                    error === "order" ||
                    error === "gap" ||
                    error === "end" ||
                    undefined
                  }
                  inputMode="numeric"
                  placeholder="0:00"
                  className="h-11 w-20 shrink-0 rounded-control border-2 border-line bg-surface px-2 text-center font-bold tabular-nums outline-none focus:border-brand aria-invalid:border-danger"
                  onChange={(event) => patch(chapter.key, { time: event.target.value })}
                />
                <input
                  value={chapter.title}
                  aria-label={t("studio.chapters.name", { index: index + 1 })}
                  aria-invalid={error === "title" || undefined}
                  maxLength={100}
                  placeholder={t("studio.chapters.namePlaceholder")}
                  className="h-11 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-3 outline-none focus:border-brand aria-invalid:border-danger"
                  onChange={(event) => patch(chapter.key, { title: event.target.value })}
                />
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("studio.chapters.remove", { index: index + 1 })}
                  onClick={() =>
                    onChange(chapters.filter((existing) => existing.key !== chapter.key))
                  }
                >
                  <X className="size-4" />
                </Button>
              </div>
              {error && (
                <p role="alert" className="mt-1 px-1 text-sm font-medium text-danger">
                  {t(`studio.chapters.errors.${error}`)}
                </p>
              )}
            </li>
          );
        })}
      </ol>
      {chapters.length < MAX_CHAPTERS && (
        <Button
          variant="secondary"
          className="self-start"
          icon={
            playerTime && chapters.length > 0 ? (
              <Clock className="size-4" />
            ) : (
              <Plus className="size-4" />
            )
          }
          onClick={addAt}
        >
          {playerTime && chapters.length > 0
            ? t("studio.chapters.addHere")
            : t("studio.chapters.add")}
        </Button>
      )}
    </>
  );
}
