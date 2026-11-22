import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { fromDate, plans, toDate } from "@tokuly/api";
import { BellRing, Crown, DoorOpen, Link2, Lock, LockOpen, MoreHorizontal, PartyPopper, Pencil, Send, Trash2, UserMinus, UserPlus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { ShareToChat } from "#/features/chat";
import { copyText, EmojiPicker } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { keys, nameOf, pollClient, rememberPlan, rememberPoll, usePoll } from "../api";
import { formatDay, formatTime, ymd } from "../dates";
import { FriendPicker } from "../friend-picker";
import { SectionTitle, useNow } from "../parts";
import { DatePicker } from "./date-picker";
import { ANSWER_MARK, buildSlots, slotDay, slotLabel, slotText, takesAnswers } from "./model";
import { PollStatusChip } from "./poll-card";

const ANSWERS = [
  { answer: plans.PollAnswer.YES, key: "yes", tone: "bg-lime text-[#17122b]" },
  { answer: plans.PollAnswer.MAYBE, key: "maybe", tone: "bg-sun text-[#17122b]" },
  { answer: plans.PollAnswer.NO, key: "no", tone: "bg-pink text-white" },
] as const;

/** `/plans/polls/<id or short id>`: answer a date poll, see everyone's answers, and turn it into a plan. */
export function PollScreen({ pollId }: { pollId: string }) {
  const { t } = useTranslation("plans");
  const { data: poll, isPending, isError } = usePoll(pollId);
  if (isPending) {
    return (
      <Page title={t("poll.title")} back="/plans" wide>
        <Skeleton className="h-40 rounded-card" />
        <Skeleton className="mt-4 h-64 rounded-card" />
      </Page>
    );
  }
  if (isError || !poll) {
    return (
      <Page title={t("poll.title")} back="/plans">
        <EmptyState
          emoji="🫥"
          title={t("poll.notFound")}
          body={t("poll.notFoundBody")}
          action={
            <Link to="/plans" search={{ tab: "polls" }} className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("plan.backToPlans")}
            </Link>
          }
        />
      </Page>
    );
  }
  return <LoadedPoll poll={poll} />;
}

function LoadedPoll({ poll }: { poll: plans.Poll }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const myId = me?.user?.id ?? "";
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const now = useNow();
  const [finalizing, setFinalizing] = useState(false);
  const [editing, setEditing] = useState(false);
  const [inviting, setInviting] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [confirming, setConfirming] = useState<{ kind: "delete" } | { kind: "leave" } | { kind: "remove"; participant: plans.PollParticipant }>();

  const open = takesAnswers(poll, now);
  const finalized = poll.status === plans.PollStatus.FINALIZED;
  const link = `${window.location.origin}/plans/polls/${poll.shortId || poll.id}`;
  const deadline = toDate(poll.deadline);
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));
  const remember = (next: plans.Poll | undefined) => rememberPoll(queryClient, next);
  const gone = () => {
    queryClient.removeQueries({ queryKey: keys.poll(poll.id) });
    queryClient.removeQueries({ queryKey: keys.poll(poll.shortId) });
    void queryClient.invalidateQueries({ queryKey: keys.polls });
    void navigate({ to: "/plans", search: { tab: "polls" } });
  };

  const close = useMutation({
    mutationFn: () => pollClient().closePoll({ pollId: poll.id }),
    onSuccess: (response) => {
      remember(response.poll);
      toast(t("poll.closed"));
    },
    onError,
  });
  const reopen = useMutation({
    mutationFn: () => pollClient().reopenPoll({ pollId: poll.id }),
    onSuccess: (response) => {
      remember(response.poll);
      toast.success(t("poll.reopened"));
    },
    onError,
  });
  const remind = useMutation({
    mutationFn: () => pollClient().remindPollParticipants({ pollId: poll.id }),
    onSuccess: (response) => toast.success(response.remindedCount > 0 ? t("poll.reminded", { count: response.remindedCount }) : t("poll.remindedNone")),
    onError,
  });
  const invite = useMutation({
    mutationFn: (userIds: string[]) => pollClient().inviteToPoll({ pollId: poll.id, userIds }),
    onSuccess: (response) => {
      remember(response.poll);
      setInviting(false);
      toast.success(response.invitedCount > 0 ? t("people.invited", { count: response.invitedCount }) : t("people.invitedNone"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (userId: string) => pollClient().removePollParticipant({ pollId: poll.id, userId }),
    onSuccess: (response) => {
      remember(response.poll);
      setConfirming(undefined);
      toast(t("people.removed"));
    },
    onError,
  });
  const leave = useMutation({
    mutationFn: () => pollClient().leavePoll({ pollId: poll.id }),
    onSuccess: () => {
      toast(t("poll.left"));
      gone();
    },
    onError,
  });
  const destroy = useMutation({
    mutationFn: () => pollClient().deletePoll({ pollId: poll.id }),
    onSuccess: () => {
      toast(t("poll.deleted"));
      gone();
    },
    onError,
  });

  const copyLink = async () => {
    if (await copyText(link)) toast.success(t("poll.linkCopied"));
  };
  const actions: MenuAction[] = [
    { label: t("poll.menu.copyLink"), icon: <Link2 />, onSelect: () => void copyLink() },
    { label: t("poll.menu.share"), icon: <Send />, onSelect: () => setSharing(true) },
    ...(poll.mine
      ? [
          ...(poll.status === plans.PollStatus.OPEN
            ? [
                { label: t("poll.menu.invite"), icon: <UserPlus />, onSelect: () => setInviting(true) },
                { label: t("poll.menu.remind"), icon: <BellRing />, onSelect: () => remind.mutate() },
                { label: t("poll.menu.edit"), icon: <Pencil />, onSelect: () => setEditing(true) },
                { label: t("poll.menu.close"), icon: <Lock />, onSelect: () => close.mutate() },
              ]
            : []),
          ...(poll.status === plans.PollStatus.CLOSED ? [{ label: t("poll.menu.reopen"), icon: <LockOpen />, onSelect: () => reopen.mutate() }] : []),
          { label: t("poll.menu.delete"), icon: <Trash2 />, danger: true, onSelect: () => setConfirming({ kind: "delete" }) },
        ]
      : [{ label: t("poll.menu.leave"), icon: <DoorOpen />, danger: true, onSelect: () => setConfirming({ kind: "leave" }) }]),
  ];

  const best = poll.slots.find((slot) => slot.id === poll.bestSlotId);
  const final = poll.slots.find((slot) => slot.id === poll.finalSlotId);

  return (
    <Page
      title={poll.title}
      back="/plans"
      wide
      actions={
        <Menu
          actions={actions}
          trigger={
            <Button variant="ghost" size="icon" aria-label={t("poll.menu.label")}>
              <MoreHorizontal className="size-6" />
            </Button>
          }
        />
      }
    >
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="flex flex-col gap-6">
          <Card>
            <div className="flex items-start gap-3">
              <span className="grid size-16 shrink-0 place-items-center rounded-2xl border-[2.5px] border-ink bg-sky text-4xl" aria-hidden>
                🗳️
              </span>
              <div className="min-w-0 flex-1">
                <h2 className="text-2xl leading-tight font-black break-words">{poll.title}</h2>
                <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                  <PollStatusChip poll={poll} />
                  <span className="text-sm font-bold text-ink-soft" data-testid="poll-answered">
                    {t("poll.answered", { answered: poll.answeredCount, total: poll.participantCount })}
                  </span>
                </div>
              </div>
            </div>
            {poll.description && <p className="mt-3 leading-relaxed break-words whitespace-pre-wrap">{poll.description}</p>}
            <p className="mt-3 flex items-center gap-2 text-sm font-bold text-ink-faint">
              <Avatar user={poll.creator} size="xs" />
              {t("poll.by", { name: nameOf(poll.creator) })}
            </p>
            {deadline && !finalized && <p className="mt-1 text-sm font-bold text-ink-soft">⏰ {t("poll.deadline", { time: `${formatDay(ymd(deadline), i18n.language)} ${formatTime(deadline, i18n.language)}` })}</p>}

            {final ? (
              <div className="mt-4 rounded-2xl border-[2.5px] border-ink bg-lime p-4 text-[#17122b]" data-testid="poll-final">
                <p className="text-sm font-black">🎉 {t("poll.finalTitle")}</p>
                <p className="text-xl font-black">{slotText(final, i18n.language)}</p>
                {poll.planId && (
                  <Link to="/plans/$planId" params={{ planId: poll.planId }} className="pressable sticker-sm mt-3 inline-flex h-10 items-center rounded-full bg-surface px-4 text-sm font-black text-ink">
                    {t("poll.openPlan")}
                  </Link>
                )}
              </div>
            ) : best ? (
              <div className="mt-4 rounded-2xl border-[2.5px] border-ink bg-sun p-4 text-[#17122b]" data-testid="poll-best">
                <p className="flex items-center gap-1 text-sm font-black">
                  <Crown className="size-4" />
                  {t("poll.bestTitle")}
                </p>
                <p className="text-xl font-black">{slotText(best, i18n.language)}</p>
                <p className="text-sm font-bold">
                  ○{best.yesCount} △{best.maybeCount} ×{best.noCount}
                </p>
              </div>
            ) : (
              <p className="mt-4 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">{t("poll.noAnswersYet")}</p>
            )}

            {poll.mine && !finalized && (
              <Button variant="lime" block className="mt-4" icon={<PartyPopper className="size-5" />} disabled={poll.answeredCount === 0} onClick={() => setFinalizing(true)}>
                {t("poll.finalize")}
              </Button>
            )}
            {poll.openToFriends && !finalized && (
              <Button variant="secondary" block className="mt-3" icon={<Link2 className="size-4" />} onClick={() => void copyLink()}>
                {t("poll.menu.copyLink")}
              </Button>
            )}
          </Card>

          <AnswerPanel key={poll.id} poll={poll} open={open} />
        </div>

        <div className="flex flex-col gap-6">
          <ResultsGrid poll={poll} />
          <section>
            <SectionTitle>{t("poll.people", { count: poll.participants.length })}</SectionTitle>
            <ul className="rounded-card bg-surface p-1.5">
              {poll.participants.map((participant) => (
                <li key={participant.user?.id} className="flex items-center gap-3 rounded-2xl px-2 py-1.5">
                  <Avatar user={participant.user} size="sm" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-bold">{nameOf(participant.user)}</p>
                    <p className="text-sm break-words text-ink-soft">{participant.comment ? `「${participant.comment}」` : participant.answers.length > 0 ? t("poll.didAnswer") : t("poll.notAnswered")}</p>
                  </div>
                  {poll.mine && participant.user?.id !== myId && (
                    <Button variant="ghost" size="icon-sm" aria-label={t("people.removeFrom", { name: nameOf(participant.user) })} onClick={() => setConfirming({ kind: "remove", participant })}>
                      <UserMinus className="size-4" />
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          </section>
        </div>
      </div>

      <FinalizeDialog poll={poll} open={finalizing} onOpenChange={setFinalizing} />
      <EditPollDialog poll={poll} open={editing} onOpenChange={setEditing} />
      <FriendPicker
        open={inviting}
        onOpenChange={setInviting}
        title={t("poll.inviteTitle")}
        exclude={poll.participants.map((participant) => participant.user?.id ?? "")}
        confirmLabel={t("people.inviteConfirm")}
        loading={invite.isPending}
        onConfirm={(users) => (users.length ? invite.mutate(users.map((user) => user.id)) : setInviting(false))}
      />
      <ShareToChat open={sharing} onOpenChange={setSharing} share={{ url: link }} title={poll.title} />
      <ConfirmDialog
        open={Boolean(confirming)}
        onOpenChange={(isOpen) => !isOpen && setConfirming(undefined)}
        title={confirming?.kind === "remove" ? t("people.removeTitle", { name: nameOf(confirming.participant.user) }) : confirming?.kind === "delete" ? t("poll.deleteTitle") : t("poll.leaveTitle")}
        description={confirming?.kind === "delete" ? t("poll.deleteBody") : confirming?.kind === "leave" ? t("poll.leaveBody") : t("poll.removeBody")}
        confirmLabel={confirming?.kind === "delete" ? t("poll.menu.delete") : confirming?.kind === "leave" ? t("poll.menu.leave") : t("people.remove")}
        danger
        loading={destroy.isPending || leave.isPending || remove.isPending}
        onConfirm={() => {
          if (confirming?.kind === "delete") destroy.mutate();
          else if (confirming?.kind === "leave") leave.mutate();
          else if (confirming?.kind === "remove") remove.mutate(confirming.participant.user?.id ?? "");
        }}
      />
    </Page>
  );
}

/**
 * The caller's own answers: one row per candidate with ○ △ × as buttons a
 * thumb can hit, a comment, and one button that sends them all.
 */
function AnswerPanel({ poll, open }: { poll: plans.Poll; open: boolean }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const mine = poll.participants.find((participant) => participant.user?.id === me?.user?.id);
  const saved: Record<string, plans.PollAnswer> = Object.fromEntries(poll.slots.filter((slot) => slot.myAnswer !== plans.PollAnswer.UNSPECIFIED).map((slot) => [slot.id, slot.myAnswer]));
  const [edited, setEdited] = useState<Record<string, plans.PollAnswer> | null>(null);
  const [comment, setComment] = useState<string | null>(null);
  const answers = edited ?? saved;
  const text = comment ?? mine?.comment ?? "";
  const changed = edited !== null || comment !== null;

  const send = useMutation({
    mutationFn: () =>
      pollClient().answerPoll({
        pollId: poll.id,
        // A candidate removed meanwhile is not answered.
        answers: poll.slots.flatMap((slot) => (answers[slot.id] ? [{ slotId: slot.id, answer: answers[slot.id]! }] : [])),
        comment: text.trim(),
      }),
    onSuccess: (response) => {
      rememberPoll(queryClient, response.poll);
      setEdited(null);
      setComment(null);
      toast.success(t("poll.answerSaved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  const pick = (slotId: string, answer: plans.PollAnswer) => {
    const next = { ...answers };
    if (next[slotId] === answer) delete next[slotId];
    else next[slotId] = answer;
    setEdited(next);
  };
  const answeredAll = poll.slots.every((slot) => answers[slot.id]);

  return (
    <section>
      <SectionTitle
        action={
          open && (
            <Button variant="ghost" size="sm" onClick={() => setEdited(Object.fromEntries(poll.slots.map((slot) => [slot.id, plans.PollAnswer.YES])))}>
              {t("poll.allYes")}
            </Button>
          )
        }
      >
        {t("poll.yourAnswers")}
      </SectionTitle>
      <ul className="flex flex-col gap-2">
        {poll.slots.map((slot) => {
          const label = slotLabel(slot, i18n.language);
          const isBest = slot.id === (poll.finalSlotId || poll.bestSlotId);
          return (
            <li key={slot.id} className={cn("flex items-center gap-2 rounded-card bg-surface p-2.5 pl-3.5", isBest && "sticker bg-sun/25")} data-testid="poll-slot" data-best={isBest || undefined}>
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-1 font-black">
                  {isBest && <Crown className="size-4 shrink-0 text-coral" aria-label={t("poll.bestTitle")} />}
                  <span className="truncate">{label.day}</span>
                </p>
                {label.time && <p className="text-sm font-bold text-ink-soft">{label.time}</p>}
                <p className="text-xs font-bold text-ink-faint">
                  ○{slot.yesCount} △{slot.maybeCount} ×{slot.noCount}
                </p>
              </div>
              <div role="radiogroup" aria-label={slotText(slot, i18n.language)} className="flex shrink-0 gap-1.5">
                {ANSWERS.map((choice) => {
                  const active = answers[slot.id] === choice.answer;
                  return (
                    // biome-ignore lint/a11y/useSemanticElements: three marks per row, not a form field.
                    <button
                      key={choice.key}
                      type="button"
                      role="radio"
                      aria-checked={active}
                      aria-label={t(`poll.answer.${choice.key}`)}
                      disabled={!open}
                      onClick={() => pick(slot.id, choice.answer)}
                      className={cn(
                        "grid size-12 place-items-center rounded-2xl border-[2.5px] text-2xl leading-none font-black transition-colors disabled:opacity-60",
                        active ? cn("border-ink shadow-pop-sm", choice.tone) : "border-line bg-surface text-ink-faint",
                      )}
                    >
                      {ANSWER_MARK[choice.answer]}
                    </button>
                  );
                })}
              </div>
            </li>
          );
        })}
      </ul>
      {open ? (
        <div className="mt-3 flex flex-col gap-3">
          <TextField aria-label={t("poll.comment")} placeholder={t("poll.commentPlaceholder")} value={text} maxLength={140} onChange={(event) => setComment(event.target.value)} />
          <Button size="lg" block loading={send.isPending} disabled={!changed && poll.answeredByMe} onClick={() => send.mutate()}>
            {poll.answeredByMe ? t("poll.updateAnswer") : t("poll.sendAnswer")}
          </Button>
          {!answeredAll && changed && <p className="px-1 text-xs font-bold text-ink-faint">{t("poll.partial")}</p>}
        </div>
      ) : (
        <p className="mt-3 px-1 text-sm font-bold text-ink-soft">{poll.status === plans.PollStatus.FINALIZED ? t("poll.finalizedHint") : t("poll.closedHint")}</p>
      )}
    </section>
  );
}

/** Everyone's answers as the classic table: a row per candidate, a column per person. */
function ResultsGrid({ poll }: { poll: plans.Poll }) {
  const { t, i18n } = useTranslation("plans");
  const answered = poll.participants.filter((participant) => participant.answers.length > 0);
  const topSlot = poll.finalSlotId || poll.bestSlotId;
  return (
    <section>
      <SectionTitle>{t("poll.everyone")}</SectionTitle>
      {answered.length === 0 ? (
        <p className="rounded-card border-2 border-dashed border-line px-4 py-6 text-center text-sm font-bold text-ink-soft">{t("poll.gridEmpty")}</p>
      ) : (
        <div className="overflow-x-auto rounded-card bg-surface">
          <table className="w-full border-separate border-spacing-0 text-center text-sm" data-testid="poll-grid">
            <thead>
              <tr>
                <th scope="col" className="sticky left-0 z-10 min-w-32 bg-surface px-3 py-2 text-left text-xs font-black text-ink-faint">
                  {t("poll.gridSlot")}
                </th>
                <th scope="col" className="px-2 py-2 text-xs font-black whitespace-nowrap text-ink-faint">
                  ○ △ ×
                </th>
                {answered.map((participant) => (
                  <th key={participant.user?.id} scope="col" className="min-w-14 px-1.5 py-2">
                    <span className="flex flex-col items-center gap-0.5">
                      <Avatar user={participant.user} size="xs" />
                      <span className="max-w-14 truncate text-[11px] font-bold">{nameOf(participant.user)}</span>
                    </span>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {poll.slots.map((slot) => {
                const label = slotLabel(slot, i18n.language);
                const top = slot.id === topSlot;
                return (
                  <tr key={slot.id} className={cn(top && "[&>*]:bg-sun/25")} data-best={top || undefined}>
                    <th scope="row" className={cn("sticky left-0 z-10 border-t-2 border-line px-3 py-2 text-left font-bold", top ? "bg-[color-mix(in_srgb,var(--sun)_25%,var(--surface))]!" : "bg-surface")}>
                      <span className="flex items-center gap-1 whitespace-nowrap">
                        {top && <Crown className="size-3.5 shrink-0 text-coral" aria-label={t("poll.bestTitle")} />}
                        {label.day}
                      </span>
                      {label.time && <span className="block text-xs whitespace-nowrap text-ink-soft">{label.time}</span>}
                    </th>
                    <td className="border-t-2 border-line px-2 py-2 text-xs font-black whitespace-nowrap">
                      {slot.yesCount} {slot.maybeCount} {slot.noCount}
                    </td>
                    {answered.map((participant) => {
                      const answer = participant.answers.find((given) => given.slotId === slot.id)?.answer ?? plans.PollAnswer.UNSPECIFIED;
                      return (
                        <td key={participant.user?.id} className="border-t-2 border-line px-1.5 py-2">
                          <span
                            role="img"
                            className={cn(
                              "mx-auto grid size-8 place-items-center rounded-xl text-lg leading-none font-black",
                              answer === plans.PollAnswer.YES && "bg-lime text-[#17122b]",
                              answer === plans.PollAnswer.MAYBE && "bg-sun text-[#17122b]",
                              answer === plans.PollAnswer.NO && "bg-pink text-white",
                              answer === plans.PollAnswer.UNSPECIFIED && "text-ink-faint",
                            )}
                            aria-label={answer === plans.PollAnswer.UNSPECIFIED ? t("poll.answer.none") : t(`poll.answer.${answer === plans.PollAnswer.YES ? "yes" : answer === plans.PollAnswer.MAYBE ? "maybe" : "no"}`)}
                          >
                            {ANSWER_MARK[answer] ?? "–"}
                          </span>
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

/** Picks the day and turns the poll into a plan: the creator hosts, everyone who said ○ or △ is invited. */
function FinalizeDialog({ poll, open, onOpenChange }: { poll: plans.Poll; open: boolean; onOpenChange(open: boolean): void }) {
  const { t, i18n } = useTranslation("plans");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [slotId, setSlotId] = useState("");
  const [emoji, setEmoji] = useState("");
  const [place, setPlace] = useState("");
  const chosen = slotId || poll.bestSlotId;

  const finalize = useMutation({
    mutationFn: () => pollClient().finalizePoll({ pollId: poll.id, slotId: chosen, emoji, place: place.trim() ? { name: place.trim() } : undefined, description: "" }),
    onSuccess: (response) => {
      rememberPoll(queryClient, response.poll);
      rememberPlan(queryClient, response.plan);
      onOpenChange(false);
      toast.success(t("poll.finalized"));
      if (response.plan) void navigate({ to: "/plans/$planId", params: { planId: response.plan.id } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("poll.finalizeTitle")}
      description={t("poll.finalizeLead")}
      footer={
        <Button variant="lime" loading={finalize.isPending} disabled={!chosen} onClick={() => finalize.mutate()}>
          {t("poll.finalizeConfirm")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <div role="radiogroup" aria-label={t("poll.finalizeSlot")} className="flex flex-col gap-1.5">
          {poll.slots.map((slot) => {
            const active = slot.id === chosen;
            return (
              // biome-ignore lint/a11y/useSemanticElements: rows with tallies, not a form field.
              <button
                key={slot.id}
                type="button"
                role="radio"
                aria-checked={active}
                onClick={() => setSlotId(slot.id)}
                className={cn("flex items-center gap-2 rounded-2xl border-[2.5px] px-3 py-2.5 text-left", active ? "border-ink bg-lime text-[#17122b]" : "border-line bg-surface")}
              >
                <span className="min-w-0 flex-1 truncate font-black">
                  {slot.id === poll.bestSlotId && "👑 "}
                  {slotText(slot, i18n.language)}
                </span>
                <span className="shrink-0 text-sm font-bold">
                  ○{slot.yesCount} △{slot.maybeCount} ×{slot.noCount}
                </span>
              </button>
            );
          })}
        </div>
        <div className="flex items-end gap-3">
          <EmojiPicker
            onPick={setEmoji}
            trigger={
              <button type="button" aria-label={t("plan.form.emoji")} className="pressable sticker grid size-12 shrink-0 place-items-center rounded-2xl bg-brand-soft text-2xl">
                {emoji || "📅"}
              </button>
            }
          />
          <TextField className="min-w-0 flex-1" label={t("plan.form.place")} placeholder={t("plan.form.placePlaceholder")} value={place} maxLength={80} onChange={(event) => setPlace(event.target.value)} />
        </div>
      </div>
    </Dialog>
  );
}

/** The creator's changes to an open poll: texts, deadline, the link, and the candidates. */
function EditPollDialog({ poll, open, onOpenChange }: { poll: plans.Poll; open: boolean; onOpenChange(open: boolean): void }) {
  const { t, i18n } = useTranslation("plans");
  const queryClient = useQueryClient();
  const deadlineInput = () => {
    const date = toDate(poll.deadline);
    return date ? `${ymd(date)}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}` : "";
  };
  const [title, setTitle] = useState(poll.title);
  const [description, setDescription] = useState(poll.description);
  const [openToFriends, setOpenToFriends] = useState(poll.openToFriends);
  const [deadline, setDeadline] = useState(deadlineInput);
  const [addDays, setAddDays] = useState<string[]>([]);
  const [removeIds, setRemoveIds] = useState<string[]>([]);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setTitle(poll.title);
      setDescription(poll.description);
      setOpenToFriends(poll.openToFriends);
      setDeadline(deadlineInput());
      setAddDays([]);
      setRemoveIds([]);
    }
  }
  // New whole days can be added here; candidates with a time keep the times they were created with.
  const dayPoll = poll.slots.every((slot) => slot.start.case === "date");

  const save = useMutation({
    mutationFn: () =>
      pollClient().updatePoll({
        pollId: poll.id,
        title: title.trim(),
        description: description.trim(),
        openToFriends,
        deadline: deadline ? fromDate(new Date(deadline)) : undefined,
        clearDeadline: !deadline,
        addSlots: buildSlots(addDays, null),
        removeSlotIds: removeIds,
      }),
    onSuccess: (response) => {
      rememberPoll(queryClient, response.poll);
      onOpenChange(false);
      toast.success(t("poll.saved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });
  const remaining = poll.slots.length - removeIds.length + addDays.length;

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("poll.editTitle")}
      footer={
        <Button loading={save.isPending} disabled={!title.trim() || remaining < 1} onClick={() => save.mutate()}>
          {t("plan.save")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField label={t("poll.form.title")} value={title} maxLength={80} onChange={(event) => setTitle(event.target.value)} />
        <TextArea label={t("poll.form.description")} value={description} maxLength={1000} onChange={(event) => setDescription(event.target.value)} />
        <TextField type="datetime-local" label={t("poll.form.deadline")} hint={t("poll.form.deadlineHint")} value={deadline} onChange={(event) => setDeadline(event.target.value)} />
        <SwitchRow label={t("poll.form.openToFriends")} hint={t("poll.form.openToFriendsHint")} checked={openToFriends} onChange={setOpenToFriends} />
        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("poll.editSlots")}</p>
          <ul className="flex flex-col gap-1">
            {poll.slots.map((slot) => {
              const removed = removeIds.includes(slot.id);
              return (
                <li key={slot.id} className="flex items-center gap-2 rounded-2xl bg-surface-2 px-3 py-1.5">
                  <span className={cn("min-w-0 flex-1 truncate font-bold", removed && "text-ink-faint line-through")}>{slotText(slot, i18n.language)}</span>
                  <Button variant="ghost" size="sm" onClick={() => setRemoveIds(removed ? removeIds.filter((id) => id !== slot.id) : [...removeIds, slot.id])}>
                    {removed ? t("poll.keepSlot") : t("poll.removeSlot")}
                  </Button>
                </li>
              );
            })}
          </ul>
          {removeIds.length > 0 && <p className="px-1 text-xs font-bold text-danger">{t("poll.removeSlotHint")}</p>}
          {dayPoll && (
            <div className="rounded-card border-2 border-line p-2">
              <p className="px-1 pb-1 text-sm font-bold text-ink-soft">{t("poll.addSlots")}</p>
              <DatePicker value={addDays} onChange={setAddDays} taken={poll.slots.map(slotDay)} />
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}
