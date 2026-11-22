import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { fromDate, school, toDate } from "@tokuly/api";
import { PenLine, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { useAccountTimeZone } from "#/lib/account-time";
import { keys, refresh, studyClient, toastError } from "./api";
import { fromLocalInput, momentLabel, studyTime, toLocalInput } from "./dates";
import { Block, RowsSkeleton, SubjectPill } from "./parts";
import { SubjectSelect } from "./subject-select";
import { CheersDialog } from "./timer-card";

/** How a session came to be: the timer, a study room, or logged by hand. */
export function sourceKey(source: school.SessionSource): "timer" | "room" | "manual" {
  if (source === school.SessionSource.MANUAL) return "manual";
  return source === school.SessionSource.ROOM ? "room" : "timer";
}

/** The study log, newest first, with "log time afterwards" and editing. */
export function StudyLog() {
  const { t, i18n } = useTranslation("school");
  const [logging, setLogging] = useState(false);
  const zone = useAccountTimeZone();
  const [editing, setEditing] = useState<school.StudySession | undefined>(undefined);
  const query = useInfiniteQuery({
    queryKey: keys.sessions,
    queryFn: ({ pageParam }) =>
      studyClient().listStudySessions({ page: { limit: 20, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const sessions = (query.data?.pages ?? [])
    .flatMap((page) => page.sessions)
    .filter((session) => session.state === school.SessionState.ENDED);

  return (
    <Block
      title={t("log.title")}
      action={
        <Button
          size="sm"
          variant="secondary"
          icon={<PenLine className="size-4" />}
          onClick={() => setLogging(true)}
        >
          {t("log.add")}
        </Button>
      }
    >
      {query.isPending ? (
        <RowsSkeleton />
      ) : sessions.length === 0 ? (
        <EmptyState emoji="📖" title={t("log.empty")} body={t("log.emptyBody")} />
      ) : (
        <ul className="flex flex-col gap-1.5" aria-label={t("log.title")}>
          {sessions.map((session) => {
            const started = toDate(session.startedAt);
            const source = sourceKey(session.source);
            return (
              <li key={session.id}>
                <button
                  type="button"
                  onClick={() => setEditing(session)}
                  className="flex w-full items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5 text-left transition-colors hover:bg-surface-3"
                >
                  <span className="min-w-0 flex-1">
                    <span className="flex flex-wrap items-center gap-1.5">
                      <SubjectPill subject={session.subject} />
                      <span
                        className={
                          source === "manual"
                            ? "rounded-full border-2 border-dashed border-ink-faint px-2 text-[11px] font-black text-ink-soft"
                            : "rounded-full bg-surface-3 px-2 py-0.5 text-[11px] font-black text-ink-soft"
                        }
                      >
                        {t(`log.source.${source}`)}
                      </span>
                      {session.cheerCount > 0 && (
                        <span className="text-xs font-bold">📣 {session.cheerCount}</span>
                      )}
                    </span>
                    <span className="mt-0.5 block truncate text-xs font-bold text-ink-soft">
                      {started && momentLabel(started, i18n.language, zone)}
                      {session.memo && ` · ${session.memo}`}
                    </span>
                  </span>
                  <span className="shrink-0 font-black">
                    {studyTime(session.studiedSeconds, i18n.language)}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <LoadMore query={query} />
      {logging && <LogDialog onClose={() => setLogging(false)} />}
      {editing && <SessionDialog session={editing} onClose={() => setEditing(undefined)} />}
    </Block>
  );
}

/** Logs study time after the fact. It counts in one's own totals, not in the friends' ranking. */
function LogDialog({ onClose }: { onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  // Times are entered on the account's clock, like every time of the school area.
  const zone = useAccountTimeZone();
  const [start, setStart] = useState(() => toLocalInput(new Date(Date.now() - 60 * 60_000), zone));
  const [end, setEnd] = useState(() => toLocalInput(new Date(), zone));
  const [subjectId, setSubjectId] = useState("");
  const [memo, setMemo] = useState("");
  const startedAt = fromLocalInput(start, zone);
  const endedAt = fromLocalInput(end, zone);
  const valid = startedAt && endedAt && endedAt.getTime() - startedAt.getTime() >= 60_000;
  const log = useMutation({
    mutationFn: () =>
      studyClient().logStudySession({
        startedAt: startedAt && fromDate(startedAt),
        endedAt: endedAt && fromDate(endedAt),
        subjectId,
        memo,
      }),
    onSuccess: () => {
      toast.success(t("log.added"));
      refresh(queryClient, "study");
      onClose();
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("log.addTitle")}
      description={t("log.addLead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={log.isPending} disabled={!valid} onClick={() => log.mutate()}>
            {t("log.addAction")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            label={t("log.start")}
            type="datetime-local"
            value={start}
            onChange={(event) => setStart(event.target.value)}
          />
          <TextField
            label={t("log.end")}
            type="datetime-local"
            value={end}
            onChange={(event) => setEnd(event.target.value)}
          />
        </div>
        <SubjectSelect value={subjectId} onChange={setSubjectId} />
        <TextField
          label={t("timer.memo")}
          value={memo}
          maxLength={200}
          placeholder={t("timer.memoPlaceholder")}
          onChange={(event) => setMemo(event.target.value)}
        />
        <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm text-ink-soft">
          {t("log.manualNote")}
        </p>
      </div>
    </Dialog>
  );
}

/**
 * A logged session: change its subject or memo, correct its times, or
 * delete it. A session the server timed can only be made shorter; one
 * logged by hand can be moved freely.
 */
function SessionDialog({ session, onClose }: { session: school.StudySession; onClose(): void }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const zone = useAccountTimeZone();
  const original = {
    start: toLocalInput(toDate(session.startedAt) ?? new Date(), zone),
    end: toLocalInput(toDate(session.endedAt) ?? new Date(), zone),
  };
  const [start, setStart] = useState(original.start);
  const [end, setEnd] = useState(original.end);
  const [subjectId, setSubjectId] = useState(session.subject?.id ?? "");
  const [memo, setMemo] = useState(session.memo);
  const [deleting, setDeleting] = useState(false);
  const [cheersOpen, setCheersOpen] = useState(false);
  const timed = session.source !== school.SessionSource.MANUAL;
  const timesChanged = start !== original.start || end !== original.end;
  const startedAt = fromLocalInput(start, zone);
  const endedAt = fromLocalInput(end, zone);
  const outside = timed && timesChanged && (start < original.start || end > original.end);
  const valid = startedAt && endedAt && endedAt.getTime() > startedAt.getTime() && !outside;

  const done = () => {
    refresh(queryClient, "study");
    onClose();
  };
  const save = useMutation({
    mutationFn: () =>
      studyClient().updateStudySession({
        sessionId: session.id,
        ...(subjectId !== (session.subject?.id ?? "") ? { subjectId } : {}),
        ...(memo !== session.memo ? { memo } : {}),
        // An untouched end keeps its seconds: only what was changed is sent as typed.
        ...(timesChanged
          ? {
              startedAt:
                start === original.start ? session.startedAt : startedAt && fromDate(startedAt),
              endedAt: end === original.end ? session.endedAt : endedAt && fromDate(endedAt),
            }
          : {}),
      }),
    onSuccess: done,
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => studyClient().deleteStudySession({ sessionId: session.id }),
    onSuccess: () => {
      toast.success(t("log.deleted"));
      done();
    },
    onError: toastError,
  });

  return (
    <>
      <Dialog
        open
        onOpenChange={(open) => !open && onClose()}
        title={t("log.editTitle")}
        description={`${t(`log.source.${sourceKey(session.source)}`)} · ${studyTime(session.studiedSeconds, i18n.language)}`}
        footer={
          <>
            <Button
              variant="ghost"
              className="mr-auto text-danger"
              icon={<Trash2 className="size-4" />}
              onClick={() => setDeleting(true)}
            >
              {t("common:actions.delete")}
            </Button>
            <Button loading={save.isPending} disabled={!valid} onClick={() => save.mutate()}>
              {t("common:actions.save")}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          <SubjectSelect value={subjectId} onChange={setSubjectId} />
          <TextField
            label={t("timer.memo")}
            value={memo}
            maxLength={200}
            onChange={(event) => setMemo(event.target.value)}
          />
          <div className="grid gap-3 sm:grid-cols-2">
            <TextField
              label={t("log.start")}
              type="datetime-local"
              value={start}
              min={timed ? original.start : undefined}
              max={timed ? original.end : undefined}
              onChange={(event) => setStart(event.target.value)}
            />
            <TextField
              label={t("log.end")}
              type="datetime-local"
              value={end}
              min={timed ? original.start : undefined}
              max={timed ? original.end : undefined}
              onChange={(event) => setEnd(event.target.value)}
              error={outside ? t("log.onlyShorter") : undefined}
            />
          </div>
          <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm text-ink-soft">
            {timed ? t("log.timedNote") : t("log.manualNote")}
          </p>
          {session.cheerCount > 0 && (
            <Button variant="secondary" onClick={() => setCheersOpen(true)}>
              📣 {t("study.cheersCount", { count: session.cheerCount })}
            </Button>
          )}
        </div>
      </Dialog>
      {cheersOpen && <CheersDialog sessionId={session.id} onClose={() => setCheersOpen(false)} />}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("log.deleteTitle")}
        description={t("log.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}
