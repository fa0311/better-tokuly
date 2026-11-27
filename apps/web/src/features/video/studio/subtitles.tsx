import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { video } from "@tokuly/api";
import { FileUp, Pencil, Plus, Star, Trash2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { videoClient, videoKeys } from "../api";
import { scriptUrl } from "../player/media-url";

/** Languages offered for a track, most likely first; the name is what the player's menu shows by default. */
const LANGUAGES: { tag: string; name: string }[] = [
  { tag: "ja", name: "日本語" },
  { tag: "en", name: "English" },
  { tag: "ko", name: "한국어" },
  { tag: "zh-Hans", name: "简体中文" },
  { tag: "zh-Hant", name: "繁體中文" },
  { tag: "es", name: "Español" },
  { tag: "pt-BR", name: "Português (Brasil)" },
  { tag: "fr", name: "Français" },
  { tag: "de", name: "Deutsch" },
  { tag: "id", name: "Bahasa Indonesia" },
  { tag: "th", name: "ไทย" },
  { tag: "vi", name: "Tiếng Việt" },
];

const SAMPLE = "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\n";

/** The subtitles tab of the editor: the video's tracks, and adding, editing, defaulting and deleting them. */
export function SubtitlesTab({ videoId }: { videoId: string }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const key = videoKeys.subtitles(videoId);
  const tracks = useQuery({
    queryKey: key,
    queryFn: async () => (await videoClient().listSubtitles({ videoId })).subtitles,
  });
  const [editing, setEditing] = useState<{ track?: video.SubtitleTrack } | null>(null);
  const [deleting, setDeleting] = useState<video.SubtitleTrack | null>(null);
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: key });
    // The watch page and the editor's preview carry the tracks too.
    void queryClient.invalidateQueries({ queryKey: videoKeys.videos });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "video" }));

  const setDefault = useMutation({
    mutationFn: ({ track, isDefault }: { track: video.SubtitleTrack; isDefault: boolean }) =>
      videoClient().updateSubtitle({ subtitleId: track.id, isDefault }),
    onSuccess: (_response, { isDefault }) => {
      refresh();
      toast.success(
        isDefault ? t("studio.subtitles.defaultSet") : t("studio.subtitles.defaultCleared"),
      );
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (track: video.SubtitleTrack) =>
      videoClient().deleteSubtitle({ subtitleId: track.id }),
    onSuccess: () => {
      setDeleting(null);
      refresh();
      toast.success(t("studio.subtitles.deleted"));
    },
    onError: (error) => {
      setDeleting(null);
      onError(error);
    },
  });
  const used = new Set(tracks.data?.map((track) => track.language));

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-3">
        <p className="min-w-0 flex-1 text-sm leading-relaxed text-ink-soft">
          {t("studio.subtitles.lead")}
        </p>
        <Button icon={<Plus className="size-4" />} onClick={() => setEditing({})}>
          {t("studio.subtitles.add")}
        </Button>
      </div>

      {tracks.isPending ? (
        <Skeleton className="h-20 rounded-card" />
      ) : tracks.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(tracks.error, { namespace: "video" })} />
      ) : tracks.data.length === 0 ? (
        <EmptyState
          emoji="💬"
          title={t("studio.subtitles.empty")}
          body={t("studio.subtitles.emptyBody")}
        />
      ) : (
        <ul className="flex flex-col gap-3">
          {tracks.data.map((track) => (
            <li
              key={track.id}
              data-testid="subtitle-track"
              className="sticker flex flex-wrap items-center gap-3 rounded-card bg-surface p-4"
            >
              <div className="min-w-0 flex-1">
                <p className="flex flex-wrap items-center gap-2 font-black">
                  <span lang={track.language}>{track.label || track.language}</span>
                  {track.isDefault && (
                    <span className="inline-flex h-6 items-center gap-1 rounded-full bg-sun px-2.5 text-xs font-black text-[#17122b]">
                      <Star className="size-3.5 fill-current" />
                      {t("studio.subtitles.default")}
                    </span>
                  )}
                </p>
                <p className="text-sm text-ink-soft">
                  {track.language}・{t("studio.subtitles.cues", { count: track.cueCount })}
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<Star className="size-4" />}
                  loading={setDefault.isPending && setDefault.variables?.track.id === track.id}
                  onClick={() => setDefault.mutate({ track, isDefault: !track.isDefault })}
                >
                  {track.isDefault
                    ? t("studio.subtitles.clearDefault")
                    : t("studio.subtitles.makeDefault")}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<Pencil className="size-4" />}
                  onClick={() => setEditing({ track })}
                >
                  {t("actions.edit")}
                </Button>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  className="text-danger"
                  aria-label={t("studio.subtitles.delete", {
                    label: track.label || track.language,
                  })}
                  onClick={() => setDeleting(track)}
                >
                  <Trash2 className="size-4" />
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <Dialog
        open={editing !== null}
        onOpenChange={(open) => !open && setEditing(null)}
        title={editing?.track ? t("studio.subtitles.editTitle") : t("studio.subtitles.addTitle")}
        className="sm:w-[min(40rem,calc(100vw-2rem))]"
      >
        {editing && (
          <SubtitleForm
            key={editing.track?.id ?? "new"}
            videoId={videoId}
            track={editing.track}
            usedLanguages={used}
            onDone={() => {
              setEditing(null);
              refresh();
            }}
            onCancel={() => setEditing(null)}
          />
        )}
      </Dialog>
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={t("studio.subtitles.confirmDelete", {
          label: deleting?.label || deleting?.language || "",
        })}
        description={t("studio.subtitles.confirmDeleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </div>
  );
}

/** Adds a track from a file or typed text, or changes an existing one's language, label, default flag and text. */
function SubtitleForm({
  videoId,
  track,
  usedLanguages,
  onDone,
  onCancel,
}: {
  videoId: string;
  track?: video.SubtitleTrack;
  usedLanguages: Set<string | undefined>;
  onDone(): void;
  onCancel(): void;
}) {
  const { t } = useTranslation("video");
  const fileInput = useRef<HTMLInputElement>(null);
  const firstFree = LANGUAGES.find((language) => !usedLanguages.has(language.tag))?.tag ?? "ja";
  const [language, setLanguage] = useState(track?.language ?? firstFree);
  const [label, setLabel] = useState(track?.label ?? "");
  const [isDefault, setIsDefault] = useState(track?.isDefault ?? usedLanguages.size === 0);
  const [content, setContent] = useState<string | null>(track ? null : "");

  // An existing track's text is fetched once, to be edited in place.
  const stored = useQuery({
    queryKey: ["video", "captions", "source", track?.url ?? ""],
    queryFn: async () => {
      const response = await fetch(scriptUrl(track?.url ?? ""));
      if (!response.ok) throw new Error(`subtitles failed (${response.status})`);
      return response.text();
    },
    enabled: Boolean(track),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const text = content ?? stored.data ?? "";
  const textChanged = content !== null && content !== (stored.data ?? "");

  const save = useMutation({
    mutationFn: async () => {
      const name =
        label.trim() || LANGUAGES.find((entry) => entry.tag === language)?.name || language;
      if (track)
        await videoClient().updateSubtitle({
          subtitleId: track.id,
          language,
          label: name,
          isDefault,
          source: textChanged ? { case: "content", value: text } : { case: undefined },
        });
      else
        await videoClient().addSubtitle({
          videoId,
          language,
          label: name,
          isDefault,
          source: { case: "content", value: text },
        });
    },
    onSuccess: () => {
      toast.success(track ? t("studio.subtitles.saved") : t("studio.subtitles.added"));
      onDone();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });
  const options = LANGUAGES.filter(
    (entry) => entry.tag === language || !usedLanguages.has(entry.tag),
  ).map((entry) => ({ value: entry.tag, label: entry.name, hint: entry.tag }));
  // A language set through the API that this list does not know stays selectable.
  if (!options.some((option) => option.value === language))
    options.unshift({ value: language, label: language, hint: language });

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (text.trim()) save.mutate();
      }}
    >
      <div className="grid gap-4 sm:grid-cols-2">
        <SelectField
          label={t("studio.subtitles.language")}
          value={language}
          onChange={setLanguage}
          options={options}
        />
        <TextField
          label={t("studio.subtitles.label")}
          value={label}
          maxLength={40}
          placeholder={LANGUAGES.find((entry) => entry.tag === language)?.name ?? language}
          hint={t("studio.subtitles.labelHint")}
          onChange={(event) => setLabel(event.target.value)}
        />
      </div>
      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            icon={<FileUp className="size-4" />}
            onClick={() => fileInput.current?.click()}
          >
            {t("studio.subtitles.chooseFile")}
          </Button>
          <span className="text-xs text-ink-faint">{t("studio.subtitles.fileHint")}</span>
        </div>
        <input
          ref={fileInput}
          type="file"
          accept=".vtt,.srt,text/vtt,application/x-subrip"
          className="hidden"
          aria-label={t("studio.subtitles.chooseFile")}
          data-testid="subtitle-file"
          onChange={async (event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (!file) return;
            if (file.size > 1024 * 1024) {
              toast.error(t("errors:SUBTITLE_TOO_LARGE"));
              return;
            }
            setContent(await file.text());
          }}
        />
        {track && stored.isPending ? (
          <Skeleton className="h-48" />
        ) : (
          <TextArea
            label={t("studio.subtitles.content")}
            value={text}
            rows={10}
            spellCheck={false}
            placeholder={SAMPLE + t("studio.subtitles.sampleLine")}
            hint={t("studio.subtitles.contentHint")}
            onChange={(event) => setContent(event.target.value)}
          />
        )}
      </div>
      <SwitchRow
        label={t("studio.subtitles.defaultSwitch")}
        hint={t("studio.subtitles.defaultHint")}
        checked={isDefault}
        onChange={setIsDefault}
      />
      <div className="flex justify-end gap-2 pb-safe">
        <Button variant="ghost" onClick={onCancel}>
          {t("actions.cancel")}
        </Button>
        <Button type="submit" disabled={!text.trim()} loading={save.isPending}>
          {track ? t("actions.save") : t("studio.subtitles.add")}
        </Button>
      </div>
    </form>
  );
}
