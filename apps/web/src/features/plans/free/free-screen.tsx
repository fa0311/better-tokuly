import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { plans, toMillis } from "@tokuly/api";
import { BellPlus, BellRing, CalendarPlus, MapPin, MessageCircle, Pencil, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { openDirectChat } from "#/features/chat";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { freeClient, keys, nameOf, setMyFreeStatus, useFreeAnswers, useFreeBoard } from "../api";
import { type PlanDraft, setPlanDraft } from "../draft";
import { FriendPicker } from "../friend-picker";
import { AvatarStack, SectionTitle, useNow } from "../parts";
import { useAnswerFree, useFreeAlerts } from "./answer";
import { PulseFace } from "./free-strip";
import { FreeFriendSheet } from "./friend-sheet";
import { badgeText, expiryText, hasEnded, isRunning, whenText } from "./model";
import { SetFreeSheet } from "./set-free-sheet";

/** `/plans/free`: the person's own status with its answers, and every friend who is free. */
export function FreeScreen() {
  const { t } = useTranslation("plans");
  const now = useNow(15_000);
  const board = useFreeBoard();
  const [setting, setSetting] = useState(false);
  const mine = board.data?.mine && !hasEnded(board.data.mine, now) ? board.data.mine : undefined;
  const statuses = (board.data?.statuses ?? []).filter(
    (status) => status.user && !hasEnded(status, now),
  );

  return (
    <Page title={t("free.title")} back="/plans" wide>
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,5fr)_minmax(0,6fr)]">
        <div className="flex flex-col gap-6">
          {board.isPending ? (
            <Skeleton className="h-48 rounded-card" />
          ) : (
            <MyStatus status={mine} now={now} onEdit={() => setSetting(true)} />
          )}
          {mine && <Answers />}
          <Alerts />
        </div>
        <section>
          <SectionTitle>{t("free.friendsTitle")}</SectionTitle>
          {board.isPending ? (
            <div className="flex flex-col gap-3">
              <Skeleton className="h-24 rounded-card" />
              <Skeleton className="h-24 rounded-card" />
            </div>
          ) : statuses.length === 0 ? (
            <Card className="p-0">
              <EmptyState emoji="🦗" title={t("free.strip.empty")} body={t("free.emptyBody")} />
            </Card>
          ) : (
            <>
              <FriendList statuses={statuses} now={now} />
              <LoadMore query={board.paging} />
            </>
          )}
        </section>
      </div>
      <SetFreeSheet open={setting} onOpenChange={setSetting} current={mine} />
    </Page>
  );
}

function MyStatus({
  status,
  now,
  onEdit,
}: {
  status: plans.FreeStatus | undefined;
  now: number;
  onEdit(): void;
}) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const end = useMutation({
    mutationFn: () => freeClient().clearFreeStatus({}),
    onSuccess: () => {
      setMyFreeStatus(queryClient, undefined);
      void queryClient.invalidateQueries({ queryKey: keys.free });
      toast(t("free.ended"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  if (!status) {
    return (
      <Card className="flex flex-col items-center gap-3 bg-brand-soft py-8 text-center">
        <span className="animate-float text-6xl">🛋️</span>
        <p className="text-xl font-black">{t("free.cta.title")}</p>
        <p className="max-w-xs text-sm font-bold text-ink-soft">{t("free.cta.body")}</p>
        <Button variant="lime" size="lg" onClick={onEdit}>
          {t("free.cta.action")}
        </Button>
      </Card>
    );
  }

  const audience =
    status.audience === plans.FreeAudience.BEST_FRIENDS
      ? t("free.audience.best")
      : status.audience === plans.FreeAudience.CHOSEN
        ? t("free.audience.chosenCount", { count: status.chosen.length })
        : t("free.audience.friends");
  return (
    <Card className="bg-lime text-[#17122b]" data-testid="my-free-status">
      <div className="flex items-center gap-4">
        <PulseFace user={me?.user} live={isRunning(status, now)} />
        <div className="min-w-0 flex-1">
          <p className="text-2xl font-black">
            {isRunning(status, now) ? t("free.mine.now") : t("free.mine.soon")}
          </p>
          <p className="font-bold">{whenText(status, now, i18n.language, t)}</p>
        </div>
      </div>
      {status.note && (
        <p className="mt-3 rounded-2xl border-2 border-[#17122b] bg-white/70 px-4 py-2.5 font-bold">
          「{status.note}」
        </p>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2 text-sm font-bold">
        <span className="rounded-full bg-[#17122b]/10 px-3 py-1">👀 {audience}</span>
        {status.audience === plans.FreeAudience.CHOSEN && <AvatarStack users={status.chosen} />}
        {status.areaName && (
          <span className="flex items-center gap-1 rounded-full bg-[#17122b]/10 px-3 py-1">
            <MapPin className="size-4" />
            {status.areaName}
          </span>
        )}
      </div>
      <p className="mt-3 text-sm font-black">⏳ {expiryText(status, now, t)}</p>
      <div className="mt-4 flex gap-2">
        <Button variant="secondary" size="sm" icon={<Pencil className="size-4" />} onClick={onEdit}>
          {t("free.mine.edit")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="text-[#17122b] hover:bg-[#17122b]/10"
          loading={end.isPending}
          onClick={() => end.mutate()}
        >
          {t("free.mine.end")}
        </Button>
      </div>
    </Card>
  );
}

/** Who answered "あそぼ!" to the person's status. */
function Answers() {
  const { t } = useTranslation("plans");
  const navigate = useNavigate();
  const now = useNow();
  const answers = useFreeAnswers();
  const planWith = (userId: string) => {
    setPlanDraft({ inviteeIds: [userId] });
    void navigate({ to: "/plans/new" });
  };

  return (
    <section>
      <SectionTitle>{t("free.answers.title")}</SectionTitle>
      {answers.isPending ? (
        <Skeleton className="h-16" />
      ) : answers.data?.length ? (
        <ul className="flex flex-col gap-2">
          {answers.data.map((answer) => (
            <li key={answer.user?.id} className="animate-pop-in rounded-card bg-surface p-3">
              <div className="flex items-center gap-3">
                <Avatar user={answer.user} outlined />
                <div className="min-w-0 flex-1">
                  <p className="truncate font-black">
                    {nameOf(answer.user)} <span className="text-pink">{t("free.asobo")}</span>
                  </p>
                  <p className="truncate text-sm text-ink-soft">
                    {answer.message || compactAge(toMillis(answer.createdAt), now)}
                  </p>
                </div>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  aria-label={t("free.friend.chat")}
                  onClick={() => void openDirectChat(answer.user?.id ?? "")}
                >
                  <MessageCircle className="size-4" />
                </Button>
                <Button
                  size="icon-sm"
                  aria-label={t("free.friend.plan")}
                  onClick={() => planWith(answer.user?.id ?? "")}
                >
                  <CalendarPlus className="size-4" />
                </Button>
              </div>
            </li>
          ))}
          <LoadMore query={answers.paging} />
        </ul>
      ) : (
        <p className="rounded-card border-2 border-dashed border-line px-4 py-5 text-center text-sm font-bold text-ink-soft">
          {t("free.answers.empty")}
        </p>
      )}
    </section>
  );
}

function FriendList({ statuses, now }: { statuses: plans.FreeStatus[]; now: number }) {
  const { t, i18n } = useTranslation("plans");
  const answer = useAnswerFree();
  const [opened, setOpened] = useState<{ userId: string; draft?: PlanDraft }>();
  const openedStatus = opened && statuses.find((status) => status.user?.id === opened.userId);

  return (
    <>
      <ul className="flex flex-col gap-3">
        {statuses.map((status) => {
          const userId = status.user?.id ?? "";
          return (
            <li
              key={userId}
              className="sticker flex items-center gap-3 rounded-card bg-surface p-3"
              data-testid="free-friend"
            >
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-3 text-left"
                onClick={() => setOpened({ userId })}
              >
                <PulseFace user={status.user} live={isRunning(status, now)} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">{nameOf(status.user)}</span>
                  <span className="mt-0.5 flex flex-wrap items-center gap-1.5">
                    <Chip className={isRunning(status, now) ? "bg-lime text-[#17122b]" : undefined}>
                      {badgeText(status, now, i18n.language, t)}
                    </Chip>
                    <span className="text-xs font-bold text-ink-soft">
                      {whenText(status, now, i18n.language, t)}
                    </span>
                  </span>
                  {status.note && (
                    <span className="mt-1 block truncate text-sm font-bold">「{status.note}」</span>
                  )}
                </span>
              </button>
              <Button
                variant={status.answeredByMe ? "secondary" : "lime"}
                size="sm"
                className="shrink-0"
                onClick={() =>
                  status.answeredByMe
                    ? setOpened({ userId })
                    : answer.mutate(
                        { status },
                        {
                          onSuccess: (response) => setOpened({ userId, draft: response.planDraft }),
                        },
                      )
                }
              >
                {status.answeredByMe ? `✓ ${t("free.asoboSent")}` : t("free.asobo")}
              </Button>
            </li>
          );
        })}
      </ul>
      <FreeFriendSheet
        status={openedStatus}
        draft={opened?.draft}
        onOpenChange={(open) => !open && setOpened(undefined)}
      />
    </>
  );
}

/** "Tell me when they are free": the friends the person asked to hear about. */
function Alerts() {
  const { t } = useTranslation("plans");
  const alerts = useFreeAlerts();
  const [picking, setPicking] = useState(false);

  return (
    <section>
      <SectionTitle
        action={
          <Button
            variant="ghost"
            size="sm"
            icon={<BellPlus className="size-4" />}
            onClick={() => setPicking(true)}
          >
            {t("free.alerts.add")}
          </Button>
        }
      >
        <span className="flex items-center gap-1.5">
          <BellRing className="size-4" />
          {t("free.alerts.title")}
        </span>
      </SectionTitle>
      {alerts.users.length === 0 ? (
        <p className="px-1 text-sm text-ink-soft">{t("free.alerts.empty")}</p>
      ) : (
        <ul className="rounded-card bg-surface p-1.5">
          {alerts.users.map((user) => (
            <li key={user.id}>
              <ListRow
                leading={<Avatar user={user} size="sm" />}
                title={nameOf(user)}
                trailing={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={t("free.alerts.remove", { name: nameOf(user) })}
                    onClick={() => alerts.set.mutate({ userIds: [user.id], enabled: false })}
                  >
                    <X className="size-4" />
                  </Button>
                }
              />
            </li>
          ))}
        </ul>
      )}
      <FriendPicker
        open={picking}
        onOpenChange={setPicking}
        title={t("free.alerts.pickTitle")}
        description={t("free.alerts.pickLead")}
        exclude={alerts.users.map((user) => user.id)}
        loading={alerts.set.isPending}
        onConfirm={(users) => {
          if (users.length === 0) return setPicking(false);
          alerts.set.mutate(
            { userIds: users.map((user) => user.id), enabled: true },
            { onSuccess: () => setPicking(false) },
          );
        }}
      />
    </section>
  );
}
