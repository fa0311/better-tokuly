import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common, engagement, type notes } from "@tokuly/api";
import { type Audience, noteAudience, notesKeys, sameAudience } from "@tokuly/core/notes";
import {
  BarChart3,
  BellOff,
  Ellipsis,
  Flag,
  Globe,
  Heart,
  History,
  Link2,
  MessageCircle,
  Paperclip,
  Pencil,
  Pin,
  PinOff,
  Quote,
  Repeat2,
  RotateCcw,
  Trash2,
  UsersRound,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import {
  AddReactionButton,
  BookmarkButton,
  compactCount,
  ReportDialog,
  ShareButton,
  type Subject,
  useBurst,
  useCopySubjectLink,
  useEngagement,
  useShareStats,
} from "#/features/engagement";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { useDeleteNote, useLike, usePin, useRenote, useSetVisibility } from "./actions";
import { noteClient } from "./api";
import { AddToClipSheet } from "./clips-sheet";
import { openCompose } from "./compose-store";
import {
  EditDialog,
  HistoryDialog,
  NoteAnalyticsPanel,
  useHideRenotes,
  useMuteThread,
  useRedraft,
} from "./note-extras";
import { VisibilityChoices } from "./visibility";

export const noteSubject = (noteId: string): Subject => ({
  type: common.SubjectType.NOTE,
  id: noteId,
});

const ACTION =
  "relative inline-flex h-9 min-w-9 items-center justify-center gap-1.5 rounded-full px-2 text-sm font-black text-ink-soft transition-colors disabled:opacity-40";

function Count({ value }: { value: bigint | undefined }) {
  const { i18n } = useTranslation();
  const count = Number(value ?? 0n);
  if (count <= 0) return null;
  return <span className="tabular-nums">{compactCount(count, i18n.language)}</span>;
}

/**
 * The row under a note: reply, renote (or quote), like, an emoji reaction,
 * save and share. Counts are the note's own and move live.
 */
export function NoteActions({ note, className }: { note: notes.Note; className?: string }) {
  const { t } = useTranslation("notes");
  const like = useLike();
  const renote = useRenote();
  const { fire, burst } = useBurst(["💗", "💖", "✨"]);
  const subject = noteSubject(note.id);
  const liked = note.viewer?.liked ?? false;
  const renoted = note.viewer?.renoted ?? false;
  // What the viewer may do is the server's call (locked accounts, audiences).
  const canRenote = note.viewer?.canRenote ?? false;
  const canQuote = note.viewer?.canQuote ?? false;
  const author = note.author?.handle ?? "";

  return (
    <div className={cn("flex items-center justify-between", className)}>
      <button
        type="button"
        aria-label={t("actions.reply")}
        onClick={() => openCompose({ replyTo: note })}
        className={cn(ACTION, "hover:bg-surface-2 hover:text-sky")}
      >
        <MessageCircle className="size-[18px]" />
        <Count value={note.counters?.replyCount} />
      </button>

      <Menu
        align="start"
        trigger={
          <button
            type="button"
            aria-label={t("actions.renote")}
            aria-pressed={renoted}
            disabled={!canRenote && !canQuote}
            title={canRenote || canQuote ? undefined : t("actions.renotePublicOnly")}
            className={cn(
              ACTION,
              renoted ? "text-success" : "hover:bg-surface-2 hover:text-success",
            )}
          >
            <Repeat2 className="size-[19px]" strokeWidth={renoted ? 3 : 2} />
            <Count value={note.counters?.renoteCount} />
          </button>
        }
        actions={[
          {
            label: renoted ? t("actions.undoRenote") : t("actions.renoteNow"),
            icon: <Repeat2 />,
            disabled: !canRenote && !renoted,
            onSelect: () => void renote(note),
          },
          {
            label: t("actions.quote"),
            icon: <Quote />,
            disabled: !canQuote,
            onSelect: () => openCompose({ quote: note }),
          },
        ]}
      />

      <button
        type="button"
        aria-label={t("actions.like")}
        aria-pressed={liked}
        onClick={() => {
          if (!liked) fire();
          void like(note);
        }}
        className={cn(ACTION, liked ? "text-pink" : "hover:bg-surface-2 hover:text-pink")}
      >
        <Heart
          className={cn(
            "size-[18px] transition-transform duration-200 ease-bounce",
            liked && "scale-115 fill-current",
          )}
        />
        <Count value={note.counters?.likeCount} />
        {burst}
      </button>

      <AddReactionButton subject={subject} />
      <BookmarkButton subject={subject} />
      <ShareButton subject={subject} title={t("share.title", { handle: author })} />
    </div>
  );
}

type Sheet =
  | "delete"
  | "visibility"
  | "likers"
  | "renoters"
  | "report"
  | "stats"
  | "edit"
  | "history"
  | "clip";

/**
 * The "more" menu of a note: copy its link, see who liked and renoted it,
 * and — on the viewer's own notes — pin it, change who sees it, look at its
 * numbers or delete it; on other people's notes, report it.
 */
export function NoteMenu({
  note,
  renoter,
  onDeleted,
}: {
  note: notes.Note;
  /** Set on a renote row: who renoted it, whose renotes can be hidden from here. */
  renoter?: common.UserSummary;
  onDeleted?(): void;
}) {
  const { t } = useTranslation("notes");
  const me = useMe();
  const [sheet, setSheet] = useState<Sheet>();
  const copyLink = useCopySubjectLink(noteSubject(note.id));
  const pin = usePin();
  const remove = useDeleteNote();
  const redraft = useRedraft();
  const muteThread = useMuteThread();
  const hideRenotes = useHideRenotes();
  const [deleting, setDeleting] = useState(false);
  const mine = Boolean(me?.user && note.author?.id === me.user.id);
  const close = () => setSheet(undefined);

  const actions: MenuAction[] = [
    { label: t("menu.copyLink"), icon: <Link2 />, onSelect: () => void copyLink() },
  ];
  if (Number(note.counters?.likeCount ?? 0n) > 0)
    actions.push({ label: t("menu.likers"), icon: <Heart />, onSelect: () => setSheet("likers") });
  if (Number(note.counters?.renoteCount ?? 0n) > 0)
    actions.push({
      label: t("menu.renoters"),
      icon: <Repeat2 />,
      onSelect: () => setSheet("renoters"),
    });
  if (note.editCount > 0)
    actions.push({
      label: t("edit.history"),
      icon: <History />,
      onSelect: () => setSheet("history"),
    });
  if (me?.user && renoter && renoter.id !== me.user.id)
    actions.push({
      label: t("mutes.hideRenotes", { name: renoter.displayName || renoter.handle }),
      icon: <Repeat2 />,
      onSelect: () => void hideRenotes(renoter.id),
    });
  if (me?.user)
    actions.push({
      label: note.viewer?.threadMuted ? t("mutes.unmuteThread") : t("mutes.muteThread"),
      icon: <BellOff />,
      onSelect: () => void muteThread(note, !note.viewer?.threadMuted),
    });
  if (me?.user)
    actions.push({ label: t("clips:menu"), icon: <Paperclip />, onSelect: () => setSheet("clip") });
  if (mine) {
    if (note.viewer?.canEdit)
      actions.push({ label: t("edit.title"), icon: <Pencil />, onSelect: () => setSheet("edit") });
    actions.push({
      label: t("edit.redraft"),
      icon: <RotateCcw />,
      onSelect: () => void redraft(note),
    });
    actions.push(
      note.pinned
        ? { label: t("menu.unpin"), icon: <PinOff />, onSelect: () => void pin(note, false) }
        : { label: t("menu.pin"), icon: <Pin />, onSelect: () => void pin(note, true) },
      { label: t("menu.visibility"), icon: <Globe />, onSelect: () => setSheet("visibility") },
      { label: t("menu.stats"), icon: <BarChart3 />, onSelect: () => setSheet("stats") },
      {
        label: t("menu.delete"),
        icon: <Trash2 />,
        danger: true,
        onSelect: () => setSheet("delete"),
      },
    );
  } else {
    actions.push({
      label: t("menu.report"),
      icon: <Flag />,
      danger: true,
      onSelect: () => setSheet("report"),
    });
  }

  return (
    <>
      <Menu
        trigger={
          <button
            type="button"
            aria-label={t("menu.label")}
            className="-my-1 -mr-1.5 grid size-8 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
          >
            <Ellipsis className="size-[18px]" />
          </button>
        }
        actions={actions}
      />
      <ConfirmDialog
        open={sheet === "delete"}
        onOpenChange={(open) => !open && close()}
        title={t("delete.title")}
        description={t("delete.body")}
        confirmLabel={t("delete.confirm")}
        danger
        loading={deleting}
        onConfirm={async () => {
          setDeleting(true);
          const deleted = await remove(note);
          setDeleting(false);
          close();
          if (deleted) onDeleted?.();
        }}
      />
      {sheet === "visibility" && <VisibilityDialog note={note} onClose={close} />}
      {sheet === "likers" && <PeopleDialog kind="likers" noteId={note.id} onClose={close} />}
      {sheet === "renoters" && <PeopleDialog kind="renoters" noteId={note.id} onClose={close} />}
      {sheet === "stats" && <StatsDialog note={note} onClose={close} />}
      {sheet === "edit" && <EditDialog note={note} onClose={close} />}
      {sheet === "history" && <HistoryDialog note={note} onClose={close} />}
      {sheet === "clip" && <AddToClipSheet note={note} onClose={close} />}
      <ReportDialog
        subject={noteSubject(note.id)}
        open={sheet === "report"}
        onOpenChange={(open) => !open && close()}
        label={t("report.label", { handle: note.author?.handle ?? "" })}
      />
    </>
  );
}

function VisibilityDialog({ note, onClose }: { note: notes.Note; onClose(): void }) {
  const { t } = useTranslation("notes");
  const setVisibility = useSetVisibility();
  const [value, setValue] = useState<Audience>(() => noteAudience(note));
  const [saving, setSaving] = useState(false);
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("visibility.changeTitle")}
      description={note.replyTo ? t("visibility.replyHint") : undefined}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={saving}>
            {t("visibility.cancel")}
          </Button>
          <Button
            disabled={sameAudience(value, noteAudience(note))}
            loading={saving}
            onClick={async () => {
              setSaving(true);
              const changed = await setVisibility(note, value);
              setSaving(false);
              if (changed) onClose();
            }}
          >
            {t("visibility.save")}
          </Button>
        </>
      }
    >
      <VisibilityChoices value={value} onChange={setValue} disabled={saving} />
    </Dialog>
  );
}

/** Who liked or renoted a note, most recent first. */
export function PeopleDialog({
  kind,
  noteId,
  onClose,
}: {
  kind: "likers" | "renoters";
  noteId: string;
  onClose(): void;
}) {
  const { t } = useTranslation("notes");
  const query = useInfiniteQuery({
    queryKey: kind === "likers" ? notesKeys.likers(noteId) : notesKeys.renoters(noteId),
    queryFn: async ({ pageParam }) => {
      const page = { cursor: pageParam, limit: 30 };
      const response =
        kind === "likers"
          ? await noteClient().listNoteLikers({ noteId, page })
          : await noteClient().listNoteRenoters({ noteId, page });
      return { users: response.users, nextCursor: response.page?.nextCursor ?? "" };
    },
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor || undefined,
  });
  const users = query.data?.pages.flatMap((page) => page.users) ?? [];

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t(`people.${kind}`)}>
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : users.length === 0 ? (
        <EmptyState emoji="🫧" title={t("people.empty")} />
      ) : (
        <ul className="flex flex-col" data-testid={kind}>
          {users.map((user) => (
            <li key={user.id}>
              <Link
                to="/u/$handle"
                params={{ handle: user.handle }}
                onClick={onClose}
                className="flex items-center gap-3 rounded-2xl px-2 py-2 hover:bg-surface-2"
              >
                <Avatar user={user} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-bold">
                    <EmojiText text={user.displayName} />
                  </span>
                  <span className="block truncate text-sm text-ink-soft">@{user.handle}</span>
                </span>
              </Link>
            </li>
          ))}
          <LoadMore query={query} />
        </ul>
      )}
    </Dialog>
  );
}

const SHARE_TARGET_KEYS: Partial<Record<engagement.ShareTarget, string>> = {
  [engagement.ShareTarget.LINK_COPY]: "linkCopy",
  [engagement.ShareTarget.CHAT]: "chat",
  [engagement.ShareTarget.EXTERNAL]: "external",
};

/** The numbers of one of the viewer's own notes: views, reactions, saves, and how it was shared. */
function StatsDialog({ note, onClose }: { note: notes.Note; onClose(): void }) {
  const { t, i18n } = useTranslation("notes");
  const subject = noteSubject(note.id);
  const { data } = useEngagement(subject);
  const shares = useShareStats(subject, true);
  const counters = note.counters;
  const number = (value: bigint | number | undefined) =>
    Number(value ?? 0).toLocaleString(i18n.language);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("stats.title")}>
      <dl className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        <Stat label={t("stats.views")} value={number(counters?.viewCount)} />
        <Stat label={t("stats.likes")} value={number(counters?.likeCount)} />
        <Stat label={t("stats.renotes")} value={number(counters?.renoteCount)} />
        <Stat label={t("stats.quotes")} value={number(counters?.quoteCount)} />
        <Stat label={t("stats.replies")} value={number(counters?.replyCount)} />
        <Stat label={t("clips:stat")} value={number(counters?.clipCount)} />
        <Stat label={t("stats.reactions")} value={number(data?.emojiTotal)} />
        <Stat label={t("stats.saves")} value={number(data?.bookmarkCount)} />
        <Stat
          label={t("stats.shares")}
          value={number(shares.data?.shareCount ?? data?.shareCount)}
        />
      </dl>
      <NoteAnalyticsPanel noteId={note.id} />
      {shares.data && shares.data.targets.length > 0 && (
        <div className="mt-4">
          <h3 className="flex items-center gap-1.5 text-sm font-black text-ink-soft">
            <UsersRound className="size-4" />
            {t("stats.shareBreakdown")}
          </h3>
          <ul className="mt-2 flex flex-col gap-1.5">
            {shares.data.targets.map((entry) => (
              <li
                key={entry.target}
                className="flex items-center justify-between rounded-xl bg-surface-2 px-3 py-2 text-sm font-bold"
              >
                <span>{t(`stats.targets.${SHARE_TARGET_KEYS[entry.target] ?? "other"}`)}</span>
                <span className="tabular-nums">{number(entry.count)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </Dialog>
  );
}

function Stat({ label, value }: { label: ReactNode; value: ReactNode }) {
  return (
    <div className="rounded-2xl bg-surface-2 px-3 py-2.5">
      <dt className="text-xs font-bold text-ink-faint">{label}</dt>
      <dd className="text-xl font-black tabular-nums">{value}</dd>
    </div>
  );
}
