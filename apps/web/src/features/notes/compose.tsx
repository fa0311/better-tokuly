import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, errorReason, media, type notes, social } from "@tokuly/api";
import { ChevronDown, EyeOff, ImagePlus, MapPin, Smile, X } from "lucide-react";
import { type ClipboardEvent, type KeyboardEvent, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Menu } from "#/components/ui/menu";
import { useSettings } from "#/features/settings";
import { StampImage, StampPicker } from "#/features/stamps";
import { api, profileClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useLocations } from "#/stores/locations";
import { useMe } from "#/stores/session";
import { postNote } from "./actions";
import { notesKeys, useHashtagSearch, useTrendingHashtags } from "./api";
import { type ComposeContext, closeCompose, useCompose } from "./compose-store";
import { discardDraft, draftKey, loadDraft, saveDraft } from "./drafts";
import { NoteBody } from "./note-card";
import { RulesAgreement, useNoteRules } from "./rules";
import { applyCompletion, type Completion, charCount, completionAt, NOTE_MAX_CHARS } from "./text";
import { UploadTile, useUploads } from "./uploads";
import { VISIBILITY_OPTIONS, visibilityOption } from "./visibility";

const friendClient = () => api().service(social.FriendService);

/** A note's place tag name may be this long. */
const PLACE_NAME_MAX = 100;
const MAX_IMAGES = 4;

/** The composer of the app. It is mounted once (with the notes background) and opened through `openCompose`. */
export function ComposeHost() {
  const open = useCompose((state) => state.open);
  const context = useCompose((state) => state.context);
  const me = useMe();
  if (!open || !me?.user) return null;
  return <Composer context={context} user={me.user} />;
}

interface PlaceDraft {
  name: string;
  lat: number;
  lng: number;
}

/** What the composer offers while a `#tag` or `@handle` is being typed. */
interface Suggestion {
  key: string;
  /** What replaces the token being typed. */
  insert: string;
  label: string;
  detail?: string;
  user?: common.UserSummary;
}

function useDebounced<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(timer);
  }, [value, delayMs]);
  return debounced;
}

/** Hashtags or people matching what is being typed at the caret. */
function useSuggestions(completion: Completion | undefined): Suggestion[] {
  const { t } = useTranslation("notes");
  const kind = completion?.kind;
  const query = useDebounced(completion?.query ?? "", 180);
  const hashtags = useHashtagSearch(query, kind === "hashtag" && query !== "");
  const trending = useTrendingHashtags();
  const people = useQuery({
    queryKey: ["notes", "mention-search", query] as const,
    queryFn: async () => {
      // Before anything is typed, friends are who people most likely mean.
      if (!query) return (await friendClient().listFriends({ page: { limit: 6 } })).friends.flatMap((friend) => (friend.user ? [friend.user] : []));
      return (await profileClient().searchUsers({ query, page: { limit: 6 } })).results.flatMap((result) => (result.user ? [result.user] : []));
    },
    enabled: kind === "mention",
    staleTime: 30_000,
  });

  if (kind === "hashtag") {
    const found = query ? (hashtags.data ?? []) : (trending.data?.hashtags.flatMap((entry) => (entry.hashtag ? [entry.hashtag] : [])) ?? []);
    return found.slice(0, 6).map((hashtag) => ({ key: hashtag.tag, insert: `#${hashtag.tag}`, label: `#${hashtag.tag}`, detail: t("compose.tagCount", { count: Number(hashtag.noteCount) }) }));
  }
  if (kind === "mention") {
    return (people.data ?? []).slice(0, 6).map((user) => ({ key: user.id, insert: `@${user.handle}`, label: user.displayName, detail: `@${user.handle}`, user }));
  }
  return [];
}

function Composer({ context, user }: { context: ComposeContext; user: common.UserSummary }) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const rules = useNoteRules();
  const settings = useSettings();
  const key = draftKey(context);
  const [draft] = useState(() => loadDraft(user.id, key));

  const [text, setText] = useState(draft?.text ?? context.text ?? "");
  const [caret, setCaret] = useState(text.length);
  const [chosenVisibility, setChosenVisibility] = useState<common.Visibility | undefined>(draft?.visibility);
  const [contentWarning, setContentWarning] = useState(draft?.contentWarning ?? false);
  const [stamp, setStamp] = useState<common.StampRef>();
  const [place, setPlace] = useState<PlaceDraft>();
  const [placeOpen, setPlaceOpen] = useState(false);
  const [restored, setRestored] = useState(Boolean(draft));
  // Only what the person wrote is worth keeping as a draft, not the text the composer opened with.
  const [touched, setTouched] = useState(false);
  const [dismissedAt, setDismissedAt] = useState<number>();
  const [active, setActive] = useState(0);
  const [idempotencyKey] = useState(() => crypto.randomUUID());
  const field = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const uploads = useUploads({
    purpose: (kind) => (kind === "video" ? media.AssetPurpose.NOTE_VIDEO : media.AssetPurpose.NOTE_IMAGE),
    maxItems: MAX_IMAGES,
    videoAlone: true,
  });

  const completion = completionAt(text, caret);
  const suggestions = useSuggestions(completion);
  const suggesting = Boolean(completion) && suggestions.length > 0 && dismissedAt !== completion?.start;
  const activeIndex = Math.min(active, suggestions.length - 1);

  // The audience: what was chosen here, else the account's default; a reply is never wider than the note it answers.
  const fallback = settings.data?.privacy?.defaultNoteVisibility || common.Visibility.PUBLIC;
  const rank = (value: common.Visibility) => VISIBILITY_OPTIONS.findIndex((option) => option.value === value);
  const wanted = chosenVisibility ?? fallback;
  const ceiling = context.replyTo?.visibility;
  const visibility = ceiling !== undefined && rank(ceiling) > rank(wanted) ? ceiling : wanted;
  const audience = visibilityOption(visibility);
  const AudienceIcon = audience.icon;

  const length = charCount(text.trim());
  const over = length > NOTE_MAX_CHARS;
  const hasContent = text.trim() !== "" || uploads.uploads.length > 0 || stamp !== undefined;
  const mustAgree = rules.data ? !rules.data.agreed : true;

  // The field grows with the text.
  useLayoutEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${element.scrollHeight}px`;
  }, [text, mustAgree]);

  // Drafts are kept as people type, a moment after they pause.
  useEffect(() => {
    if (!touched) return;
    const timer = setTimeout(() => saveDraft(user.id, key, { text, visibility: chosenVisibility, contentWarning }), 500);
    return () => clearTimeout(timer);
  }, [touched, user.id, key, text, chosenVisibility, contentWarning]);

  const close = () => {
    if (touched) {
      saveDraft(user.id, key, { text, visibility: chosenVisibility, contentWarning });
      if (text.trim()) toast(t("compose.draftSaved"));
    }
    closeCompose();
  };

  const post = useMutation({
    mutationFn: () =>
      postNote(queryClient, {
        text: text.trim(),
        assetIds: uploads.assetIds,
        stampId: stamp?.id ?? "",
        place: place ? { name: place.name, location: { lat: place.lat, lng: place.lng } } : undefined,
        visibility: chosenVisibility ?? common.Visibility.UNSPECIFIED,
        replyToId: context.replyTo?.id ?? "",
        quoteOfId: context.quote?.id ?? "",
        contentWarning,
        idempotencyKey,
      }),
    onSuccess: (note) => {
      discardDraft(user.id, key);
      closeCompose();
      toast.success(t(context.replyTo ? "toast.replied" : "toast.posted"), {
        action: { label: t("toast.view"), onClick: () => void navigate({ to: "/notes/$noteId", params: { noteId: note.id } }) },
      });
    },
    onError: (error) => {
      // The rules changed since they were agreed to: the agreement comes up again.
      if (errorReason(error) === "RULES_NOT_ACCEPTED") void queryClient.invalidateQueries({ queryKey: notesKeys.rules });
      toast.error(errorMessage(error));
    },
  });
  const canPost = hasContent && !over && !uploads.busy && !post.isPending;

  const accept = (suggestion: Suggestion) => {
    if (!completion) return;
    const next = applyCompletion(text, completion, suggestion.insert);
    setText(next.text);
    setTouched(true);
    setCaret(next.caret);
    setActive(0);
    requestAnimationFrame(() => {
      field.current?.focus();
      field.current?.setSelectionRange(next.caret, next.caret);
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // Keys pressed while a Japanese conversion is under way belong to the input method.
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (suggesting) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        setActive((activeIndex + (event.key === "ArrowDown" ? 1 : suggestions.length - 1)) % suggestions.length);
        return;
      }
      if ((event.key === "Enter" && !event.metaKey && !event.ctrlKey) || event.key === "Tab") {
        event.preventDefault();
        accept(suggestions[activeIndex]!);
        return;
      }
      if (event.key === "Escape") {
        // Closes the suggestions, not the composer.
        event.stopPropagation();
        setDismissedAt(completion?.start);
        return;
      }
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && canPost) {
      event.preventDefault();
      post.mutate();
    }
  };

  const onPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = [...event.clipboardData.files];
    if (files.length === 0) return;
    event.preventDefault();
    uploads.add(files);
  };

  const title = mustAgree ? t("rules.title") : context.replyTo ? t("compose.title.reply") : context.quote ? t("compose.title.quote") : t("compose.title.new");
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && close()}
      title={title}
      className="sm:w-[min(36rem,calc(100vw-2rem))]"
      footer={
        mustAgree ? undefined : (
          <>
            <span className={cn("mr-auto self-center text-sm font-black tabular-nums", over ? "text-danger" : length > NOTE_MAX_CHARS - 20 ? "text-coral" : "text-ink-faint")} aria-live="polite">
              {length}/{NOTE_MAX_CHARS}
            </span>
            <Button disabled={!canPost} loading={post.isPending} onClick={() => post.mutate()}>
              {context.replyTo ? t("compose.reply") : t("compose.post")}
            </Button>
          </>
        )
      }
    >
      {mustAgree ? (
        <RulesAgreement rules={rules.data} error={rules.error} onRetry={() => void rules.refetch()} onCancel={closeCompose} />
      ) : (
        <div
          data-testid="composer"
          onDragOver={(event) => event.preventDefault()}
          onDrop={(event) => {
            event.preventDefault();
            uploads.add(event.dataTransfer.files);
          }}
        >
          {context.replyTo && <ContextNote note={context.replyTo} label={t("compose.replyingTo", { handle: context.replyTo.author?.handle ?? "" })} />}
          {restored && (
            <p className="mb-3 flex items-center gap-2 rounded-xl bg-brand-soft px-3 py-2 text-sm font-bold">
              <span className="min-w-0 flex-1">{t("compose.draftRestored")}</span>
              <button
                type="button"
                className="shrink-0 rounded-full px-2 py-0.5 text-brand hover:underline"
                onClick={() => {
                  setText(context.text ?? "");
                  setRestored(false);
                  discardDraft(user.id, key);
                }}
              >
                {t("compose.discardDraft")}
              </button>
            </p>
          )}

          <div className="flex gap-3">
            <Avatar user={user} />
            <textarea
              ref={field}
              value={text}
              rows={3}
              autoFocus
              aria-label={t("compose.placeholder")}
              placeholder={context.replyTo ? t("compose.replyPlaceholder") : t("compose.placeholder")}
              onChange={(event) => {
                setText(event.target.value);
                setTouched(true);
                setCaret(event.target.selectionStart);
                setDismissedAt(undefined);
                setActive(0);
              }}
              onSelect={(event) => setCaret(event.currentTarget.selectionStart)}
              onKeyDown={onKeyDown}
              onPaste={onPaste}
              className="min-h-24 min-w-0 flex-1 resize-none bg-transparent py-2 text-[17px] leading-relaxed outline-none placeholder:text-ink-faint"
            />
          </div>

          {suggesting && (
            <div role="listbox" aria-label={t("compose.suggestions")} className="sticker-sm mt-1 overflow-hidden rounded-2xl bg-surface">
              {suggestions.map((suggestion, index) => (
                <button
                  key={suggestion.key}
                  type="button"
                  role="option"
                  aria-selected={index === activeIndex}
                  // Keep the caret in the text field while a suggestion is picked.
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => accept(suggestion)}
                  className={cn("flex w-full items-center gap-2.5 px-3 py-2 text-left", index === activeIndex ? "bg-brand-soft" : "hover:bg-surface-2")}
                >
                  {suggestion.user && <Avatar user={suggestion.user} size="sm" />}
                  <span className="min-w-0 flex-1 truncate font-black">{suggestion.label}</span>
                  <span className="shrink-0 text-sm text-ink-faint">{suggestion.detail}</span>
                </button>
              ))}
            </div>
          )}

          {contentWarning && (
            <p className="mt-3 flex items-center gap-2 rounded-xl border-2 border-dashed border-line px-3 py-2 text-sm font-bold text-ink-soft">
              <EyeOff className="size-4 shrink-0" />
              {t("compose.sensitiveOn")}
            </p>
          )}

          {uploads.uploads.length > 0 && (
            <div className={cn("mt-3 grid gap-2", uploads.uploads.length === 1 ? "grid-cols-1" : "grid-cols-2")}>
              {uploads.uploads.map((upload) => (
                <UploadTile key={upload.key} upload={upload} onRemove={() => uploads.remove(upload.key)} className={uploads.uploads.length === 1 ? "aspect-video" : "aspect-square"} />
              ))}
            </div>
          )}

          {stamp && (
            <div className="relative mt-3 w-fit">
              <StampImage stamp={stamp} size={112} />
              <button type="button" aria-label={t("compose.removeStamp")} onClick={() => setStamp(undefined)} className="absolute -top-1 -right-2 grid size-7 place-items-center rounded-full bg-ink text-surface">
                <X className="size-4" />
              </button>
            </div>
          )}

          {context.quote && <ContextNote note={context.quote} className="mt-3" />}

          {place && !placeOpen && (
            <p className="mt-3 flex w-fit max-w-full items-center gap-1 rounded-full bg-surface-2 py-1 pr-1 pl-2.5 text-sm font-black">
              <MapPin className="size-4 shrink-0 text-coral" />
              <span className="truncate">{place.name}</span>
              <button type="button" aria-label={t("compose.removePlace")} onClick={() => setPlace(undefined)} className="grid size-6 shrink-0 place-items-center rounded-full hover:bg-surface-3">
                <X className="size-3.5" />
              </button>
            </p>
          )}
          {placeOpen && (
            <PlaceForm
              onCancel={() => setPlaceOpen(false)}
              onSave={(next) => {
                setPlace(next);
                setPlaceOpen(false);
              }}
            />
          )}

          <div className="mt-3 flex flex-wrap items-center gap-1 border-t-2 border-line pt-3">
            <input
              ref={picker}
              type="file"
              accept="image/*,video/*"
              multiple
              hidden
              data-testid="note-files"
              onChange={(event) => {
                uploads.add(event.target.files ?? []);
                event.target.value = "";
              }}
            />
            <ToolButton label={t("compose.addMedia")} disabled={!uploads.canAdd} onClick={() => picker.current?.click()}>
              <ImagePlus className="size-5" />
            </ToolButton>
            <StampPicker
              onPick={setStamp}
              trigger={
                <button type="button" aria-label={t("compose.addStamp")} className="grid size-10 place-items-center rounded-full text-brand hover:bg-brand-soft">
                  <Smile className="size-5" />
                </button>
              }
            />
            <ToolButton label={t("compose.addPlace")} pressed={Boolean(place) || placeOpen} onClick={() => setPlaceOpen(!placeOpen)}>
              <MapPin className="size-5" />
            </ToolButton>
            <ToolButton label={t("compose.sensitive")} pressed={contentWarning} onClick={() => setContentWarning(!contentWarning)}>
              <EyeOff className="size-5" />
            </ToolButton>
            <span className="flex-1" />
            <Menu
              trigger={
                <button type="button" aria-label={t("visibility.title")} className="inline-flex h-9 items-center gap-1.5 rounded-full border-2 border-line px-3 text-sm font-black hover:border-ink-faint">
                  <AudienceIcon className="size-4" />
                  {t(`visibility.${audience.key}.label`)}
                  <ChevronDown className="size-4 text-ink-faint" />
                </button>
              }
              actions={VISIBILITY_OPTIONS.map((option) => {
                const Icon = option.icon;
                return {
                  label: t(`visibility.${option.key}.label`),
                  icon: <Icon />,
                  // A reply cannot be shown more widely than the note it answers.
                  disabled: ceiling !== undefined && rank(option.value) < rank(ceiling),
                  onSelect: () => setChosenVisibility(option.value),
                };
              })}
            />
          </div>
        </div>
      )}
    </Dialog>
  );
}

function ToolButton({ label, pressed, disabled, onClick, children }: { label: string; pressed?: boolean; disabled?: boolean; onClick(): void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
      className={cn("grid size-10 place-items-center rounded-full text-brand transition-colors hover:bg-brand-soft disabled:opacity-40", pressed && "bg-brand text-on-brand hover:bg-brand")}
    >
      {children}
    </button>
  );
}

/** The note being answered or quoted, small, above or below the text field. */
function ContextNote({ note, label, className }: { note: notes.Note; label?: string; className?: string }) {
  return (
    <div className={cn("mb-3 rounded-2xl border-2 border-line bg-surface-2/60 p-3", className)}>
      {label && <p className="mb-1.5 text-xs font-black text-brand">{label}</p>}
      <div className="mb-1 flex items-center gap-1.5">
        <Avatar user={note.author} size="xs" />
        <span className="truncate text-sm font-black">{note.author?.displayName}</span>
        <span className="truncate text-xs text-ink-faint">@{note.author?.handle}</span>
      </div>
      <NoteBody note={note} embedded />
    </div>
  );
}

/** Names the place a note is tagged with; its coordinates are where the device is right now. */
function PlaceForm({ onSave, onCancel }: { onSave(place: PlaceDraft): void; onCancel(): void }) {
  const { t } = useTranslation("notes");
  const [name, setName] = useState("");
  const [locating, setLocating] = useState(false);

  const save = () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const known = useLocations.getState().own;
    if (known) {
      onSave({ name: trimmed, lat: known.lat, lng: known.lng });
      return;
    }
    setLocating(true);
    navigator.geolocation.getCurrentPosition(
      (position) => onSave({ name: trimmed, lat: position.coords.latitude, lng: position.coords.longitude }),
      () => {
        setLocating(false);
        toast.error(t("compose.placeNoLocation"));
      },
      { maximumAge: 60_000, timeout: 10_000 },
    );
  };

  return (
    <div className="mt-3 rounded-2xl bg-surface-2 p-3">
      <label className="flex h-11 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand">
        <MapPin className="size-5 shrink-0 text-coral" />
        <input
          value={name}
          maxLength={PLACE_NAME_MAX}
          // biome-ignore lint/a11y/noAutofocus: the form opens to be typed into.
          autoFocus
          placeholder={t("compose.placePlaceholder")}
          aria-label={t("compose.placePlaceholder")}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              save();
            }
          }}
          className="min-w-0 flex-1 bg-transparent font-bold outline-none placeholder:font-medium placeholder:text-ink-faint"
        />
      </label>
      <p className="mt-1.5 px-1 text-xs text-ink-faint">{t("compose.placeHint")}</p>
      <div className="mt-2 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          {t("compose.cancel")}
        </Button>
        <Button variant="secondary" size="sm" disabled={!name.trim()} loading={locating} onClick={save}>
          {t("compose.placeSave")}
        </Button>
      </div>
    </div>
  );
}
