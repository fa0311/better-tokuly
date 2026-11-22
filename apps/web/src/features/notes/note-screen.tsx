import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useRouter } from "@tanstack/react-router";
import { errorReason, isNotFound, type notes } from "@tokuly/api";
import { ImagePlus, SendHorizontal } from "lucide-react";
import { type KeyboardEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { compactCount } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { postNote } from "./actions";
import { notesKeys, useDeletedNotes, useIsDeleted, useQuotes, useThread } from "./api";
import { openCompose } from "./compose-store";
import { PeopleDialog } from "./note-actions";
import { NoteCard } from "./note-card";
import { useNoteRules } from "./rules";
import { NotesPage } from "./shell";
import { charCount, NOTE_MAX_CHARS } from "./text";
import { TimelineSkeleton } from "./timeline";

type ThreadTab = "replies" | "quotes";

/** A note in its conversation: what it answers above it, the note itself, then its replies and the notes that quote it. */
export function NoteScreen({ noteId }: { noteId: string }) {
  const { t } = useTranslation("notes");
  const navigate = useNavigate();
  const router = useRouter();
  const thread = useThread(noteId);
  const gone = useIsDeleted(noteId);
  const deleted = useDeletedNotes((state) => state.ids);
  const [tab, setTab] = useState<ThreadTab>("replies");
  const focus = useRef<HTMLDivElement>(null);

  const first = thread.data?.pages[0];
  const note = first?.note;
  const ancestors = first?.ancestors.filter((ancestor) => !deleted.has(ancestor.id)) ?? [];
  const replies = (thread.data?.pages.flatMap((page) => page.replies) ?? []).filter((reply) => !deleted.has(reply.id));

  // The note the page is about starts at the top; what it answers is a scroll up away.
  const loadedId = note?.id;
  const hasAncestors = ancestors.length > 0;
  useLayoutEffect(() => {
    if (loadedId && hasAncestors) focus.current?.scrollIntoView({ block: "start" });
  }, [loadedId, hasAncestors]);

  const leave = () => (router.history.canGoBack() ? router.history.back() : void navigate({ to: "/notes" }));

  if (thread.isPending) {
    return (
      <NotesPage title={t("thread.title")} back="/notes" scrollId={`note-${noteId}`}>
        <Skeleton className="h-44 rounded-[1.25rem]" />
        <div className="mt-4">
          <TimelineSkeleton count={2} />
        </div>
      </NotesPage>
    );
  }
  if (!note || gone) {
    const missing = gone || isNotFound(thread.error);
    return (
      <NotesPage title={t("thread.title")} back="/notes" scrollId={`note-${noteId}`}>
        <EmptyState
          emoji={missing ? "🫥" : "🌧️"}
          title={missing ? t("thread.notFound") : errorMessage(thread.error)}
          body={missing ? t("thread.notFoundBody") : undefined}
          action={
            <Link to="/notes" className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("thread.toTimeline")}
            </Link>
          }
        />
      </NotesPage>
    );
  }

  const replyCount = Number(note.counters?.replyCount ?? 0n);
  const quoteCount = Number(note.counters?.quoteCount ?? 0n);
  return (
    <NotesPage title={t("thread.title")} back="/notes" scrollId={`note-${noteId}`} onRefresh={() => thread.refetch()}>
      {ancestors.map((ancestor) => (
        <div key={ancestor.id}>
          <NoteCard note={ancestor} />
          <div aria-hidden className="ml-[2.1rem] h-3 w-0.5 bg-line" />
        </div>
      ))}
      <div ref={focus} className="scroll-mt-20">
        <NoteCard note={note} variant="focus" onDeleted={leave} />
      </div>
      <FocusStats note={note} onShowQuotes={() => setTab("quotes")} />
      <ReplyBox note={note} />

      <Tabs<ThreadTab>
        className="mt-5 mb-3"
        value={tab}
        onChange={setTab}
        items={[
          { value: "replies", label: replyCount > 0 ? t("thread.repliesCount", { count: replyCount }) : t("thread.replies") },
          { value: "quotes", label: quoteCount > 0 ? t("thread.quotesCount", { count: quoteCount }) : t("thread.quotes") },
        ]}
      />
      {tab === "replies" ? (
        replies.length === 0 ? (
          <EmptyState emoji="💭" title={t("thread.noReplies")} body={t("thread.noRepliesBody")} />
        ) : (
          <div className="flex flex-col gap-2.5" data-testid="replies">
            {replies.map((reply) => (
              <NoteCard key={reply.id} note={reply} hideReplyContext />
            ))}
            <LoadMore query={thread} />
          </div>
        )
      ) : (
        <Quotes noteId={note.id} />
      )}
    </NotesPage>
  );
}

/** The numbers under the note, each leading to who or what is behind it. */
function FocusStats({ note, onShowQuotes }: { note: notes.Note; onShowQuotes(): void }) {
  const { t, i18n } = useTranslation("notes");
  const [people, setPeople] = useState<"likers" | "renoters">();
  const counters = note.counters;
  const stats = [
    { key: "likes", count: Number(counters?.likeCount ?? 0n), onClick: () => setPeople("likers") },
    { key: "renotes", count: Number(counters?.renoteCount ?? 0n), onClick: () => setPeople("renoters") },
    { key: "quotes", count: Number(counters?.quoteCount ?? 0n), onClick: onShowQuotes },
    { key: "views", count: Number(counters?.viewCount ?? 0n), onClick: undefined },
  ].filter((stat) => stat.count > 0);
  if (stats.length === 0) return null;

  return (
    <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 px-2 text-sm text-ink-soft">
      {stats.map((stat) => {
        const body = (
          <>
            <span className="font-black text-ink tabular-nums">{compactCount(stat.count, i18n.language)}</span> {t(`thread.stats.${stat.key}`)}
          </>
        );
        return stat.onClick ? (
          <button key={stat.key} type="button" onClick={stat.onClick} className="rounded-md hover:underline">
            {body}
          </button>
        ) : (
          <span key={stat.key}>{body}</span>
        );
      })}
      {people && <PeopleDialog kind={people} noteId={note.id} onClose={() => setPeople(undefined)} />}
    </div>
  );
}

/**
 * A quick reply right under the note: type and send. Pictures, stamps and
 * the rest are a tap away in the full composer, which is also where the
 * posting rules are agreed to the first time.
 */
function ReplyBox({ note }: { note: notes.Note }) {
  const { t } = useTranslation("notes");
  const me = useMe();
  const queryClient = useQueryClient();
  const rules = useNoteRules();
  const [text, setText] = useState("");
  const [idempotencyKey, setIdempotencyKey] = useState(() => crypto.randomUUID());
  const field = useRef<HTMLTextAreaElement>(null);
  const length = charCount(text.trim());
  const over = length > NOTE_MAX_CHARS;
  const agreed = rules.data?.agreed ?? false;

  useEffect(() => {
    const element = field.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${element.scrollHeight}px`;
  }, [text]);

  const send = useMutation({
    mutationFn: () => postNote(queryClient, { text: text.trim(), replyToId: note.id, idempotencyKey }),
    onSuccess: () => {
      setText("");
      setIdempotencyKey(crypto.randomUUID());
      toast.success(t("toast.replied"));
    },
    onError: (error) => {
      if (errorReason(error) === "RULES_NOT_ACCEPTED") void queryClient.invalidateQueries({ queryKey: notesKeys.rules });
      toast.error(errorMessage(error));
    },
  });
  const canSend = text.trim() !== "" && !over && !send.isPending;
  const openFull = () => openCompose({ replyTo: note, text });

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && canSend) {
      event.preventDefault();
      send.mutate();
    }
  };

  if (!me?.user) return null;
  return (
    <div className="mt-3 flex items-end gap-2.5 rounded-[1.25rem] border-2 border-line bg-surface p-2.5 focus-within:border-brand" data-testid="reply-box">
      <Avatar user={me.user} size="sm" className="mb-0.5" />
      <textarea
        ref={field}
        rows={1}
        value={text}
        readOnly={!agreed}
        aria-label={t("thread.replyPlaceholder", { handle: note.author?.handle ?? "" })}
        placeholder={t("thread.replyPlaceholder", { handle: note.author?.handle ?? "" })}
        // Before the rules are agreed to, the field leads to the composer, which shows them.
        onFocus={agreed ? undefined : openFull}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
        className="max-h-40 min-w-0 flex-1 resize-none bg-transparent py-1.5 text-[15px] leading-relaxed outline-none placeholder:text-ink-faint"
      />
      {length > NOTE_MAX_CHARS - 20 && <span className={cn("mb-2 text-xs font-black tabular-nums", over ? "text-danger" : "text-coral")}>{NOTE_MAX_CHARS - length}</span>}
      <button type="button" aria-label={t("thread.replyWithMore")} onClick={openFull} className="grid size-9 shrink-0 place-items-center rounded-full text-brand hover:bg-brand-soft">
        <ImagePlus className="size-5" />
      </button>
      <Button size="icon-sm" aria-label={t("thread.sendReply")} disabled={!canSend} loading={send.isPending} onClick={() => send.mutate()}>
        {!send.isPending && <SendHorizontal className="size-4" />}
      </Button>
    </div>
  );
}

function Quotes({ noteId }: { noteId: string }) {
  const { t } = useTranslation("notes");
  const query = useQuotes(noteId);
  const deleted = useDeletedNotes((state) => state.ids);
  const quotes = (query.data?.pages.flatMap((page) => page.notes) ?? []).filter((quote) => !deleted.has(quote.id));
  if (query.isPending) return <TimelineSkeleton count={2} />;
  if (quotes.length === 0) return <EmptyState emoji="🗯️" title={t("thread.noQuotes")} body={t("thread.noQuotesBody")} />;
  return (
    <div className="flex flex-col gap-2.5" data-testid="quotes">
      {quotes.map((quote) => (
        <NoteCard key={quote.id} note={quote} />
      ))}
      <LoadMore query={query} />
    </div>
  );
}
