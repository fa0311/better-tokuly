import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type notes, toMillis } from "@tokuly/api";
import { charCount, NOTE_MAX_CHARS, notesKeys, patchNote } from "@tokuly/core/notes";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { errorMessage } from "#/lib/errors";
import { markNoteDeleted, muteClient, noteClient } from "./api";
import { NoteRichText } from "./rich-text";
import { noteDateTime } from "./time";

/** Editing a note's words within the hour after posting. */
export function EditDialog({ note, onClose }: { note: notes.Note; onClose(): void }) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const [text, setText] = useState(note.text);
  const [warning, setWarning] = useState(note.contentWarningText);
  const save = useMutation({
    mutationFn: () =>
      noteClient().editNote({
        noteId: note.id,
        text,
        contentWarning: note.contentWarning,
        contentWarningText: warning,
        attachmentDetails: note.attachmentDetails.map(({ assetId, altText, sensitive }) => ({
          assetId,
          altText,
          sensitive,
        })),
      }),
    onSuccess: (response) => {
      const updated = response.note;
      if (updated)
        patchNote(queryClient, note.id, (current) => ({ ...updated, counters: current.counters }));
      void queryClient.invalidateQueries({ queryKey: notesKeys.edits(note.id) });
      toast.success(t("edit.saved"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "notes" })),
  });
  const over = charCount(text.trim()) > NOTE_MAX_CHARS;
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("edit.title")}
      description={t("edit.hint")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("visibility.cancel")}
          </Button>
          <Button
            disabled={over || (text === note.text && warning === note.contentWarningText)}
            loading={save.isPending}
            onClick={() => save.mutate()}
          >
            {t("edit.save")}
          </Button>
        </>
      }
    >
      {note.contentWarning && (
        <input
          value={warning}
          maxLength={100}
          aria-label={t("compose.warningText")}
          placeholder={t("compose.warningPlaceholder")}
          onChange={(event) => setWarning(event.target.value)}
          className="mb-2 h-10 w-full rounded-xl border-2 border-line bg-surface px-3 text-sm font-bold outline-none focus:border-ink"
        />
      )}
      <textarea
        value={text}
        aria-label={t("edit.title")}
        onChange={(event) => setText(event.target.value)}
        rows={6}
        className="w-full resize-y rounded-xl border-2 border-line bg-surface p-3 outline-none focus:border-ink"
      />
    </Dialog>
  );
}

/** The earlier versions of an edited note, newest first. */
export function HistoryDialog({ note, onClose }: { note: notes.Note; onClose(): void }) {
  const { t, i18n } = useTranslation("notes");
  const query = useQuery({
    queryKey: notesKeys.edits(note.id),
    queryFn: async () => (await noteClient().listNoteEdits({ noteId: note.id })).edits,
  });
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("edit.history")}>
      <ol className="flex flex-col gap-3" data-testid="note-history">
        {(query.data ?? []).map((edit) => (
          <li key={edit.version} className="rounded-2xl bg-surface-2 px-3 py-2.5">
            <p className="mb-1 text-xs font-bold text-ink-faint">
              {t("edit.version", { version: edit.version })} ·{" "}
              {t("edit.replacedAt", {
                time: noteDateTime(toMillis(edit.replacedAt), i18n.language),
              })}
            </p>
            {edit.contentWarningText && (
              <p className="mb-1 text-sm font-bold text-ink-soft">{edit.contentWarningText}</p>
            )}
            <NoteRichText
              note={{ text: edit.text, body: edit.body, mentions: [] }}
              className="text-[15px]"
            />
          </li>
        ))}
      </ol>
    </Dialog>
  );
}

/** Deletes one of the viewer's notes and keeps its content as a draft (削除して再編集). */
export function useRedraft(): (note: notes.Note) => Promise<void> {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { t } = useTranslation("notes");
  return async (note) => {
    try {
      await noteClient().redraftNote({ noteId: note.id });
      markNoteDeleted(note.id);
      void queryClient.invalidateQueries({ queryKey: notesKeys.drafts });
      toast.success(t("edit.redrafted"), {
        action: {
          label: t("toast.toDrafts"),
          onClick: () => void navigate({ to: "/notes/drafts" }),
        },
      });
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
    }
  };
}

/** Mutes or unmutes the conversation a note is in: no notifications from it. */
export function useMuteThread(): (note: notes.Note, muted: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  const { t } = useTranslation("notes");
  return async (note, muted) => {
    try {
      if (muted) await muteClient().muteThread({ noteId: note.id });
      else await muteClient().unmuteThread({ noteId: note.id });
      patchNote(queryClient, note.id, (current) =>
        current.viewer
          ? { ...current, viewer: { ...current.viewer, threadMuted: muted } }
          : current,
      );
      void queryClient.invalidateQueries({ queryKey: notesKeys.mutes });
      toast.success(t(muted ? "mutes.threadMuted" : "mutes.threadUnmuted"));
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
    }
  };
}

/** Impressions by hour over the last two days, and the engagement rate. Author only. */
export function NoteAnalyticsPanel({ noteId }: { noteId: string }) {
  const { t, i18n } = useTranslation("notes");
  const query = useQuery({
    queryKey: notesKeys.analytics(noteId),
    queryFn: () => noteClient().getNoteAnalytics({ noteId }),
  });
  const data = query.data;
  if (!data) return null;
  const hours = data.impressionsByHour;
  const highest = Math.max(1, ...hours.map((hour) => Number(hour.count)));
  return (
    <section className="mt-4" data-testid="note-analytics">
      <h3 className="text-sm font-black text-ink-soft">{t("stats.byHour")}</h3>
      <p className="text-sm text-ink-soft">
        {t("stats.engagementRate", {
          rate: (data.engagementRate * 100).toLocaleString(i18n.language, {
            maximumFractionDigits: 1,
          }),
        })}
      </p>
      {hours.length === 0 ? (
        <p className="mt-2 text-sm text-ink-faint">{t("stats.noImpressions")}</p>
      ) : (
        <div className="mt-2 flex h-24 items-end gap-0.5" role="img" aria-label={t("stats.byHour")}>
          {hours.map((hour) => (
            <span
              key={toMillis(hour.hour)}
              title={`${noteDateTime(toMillis(hour.hour), i18n.language)}: ${hour.count}`}
              className="min-w-1 flex-1 rounded-t bg-brand"
              style={{ height: `${(Number(hour.count) / highest) * 100}%` }}
            />
          ))}
        </div>
      )}
    </section>
  );
}

/** Hides the renotes of one person from the viewer's timelines. */
export function useHideRenotes(): (userId: string) => Promise<void> {
  const queryClient = useQueryClient();
  const { t } = useTranslation("notes");
  return async (userId) => {
    try {
      await muteClient().hideRenotesFrom({ userId });
      void queryClient.invalidateQueries({ queryKey: notesKeys.timelines });
      void queryClient.invalidateQueries({ queryKey: notesKeys.mutes });
      toast.success(t("mutes.renotesHidden"));
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
    }
  };
}
