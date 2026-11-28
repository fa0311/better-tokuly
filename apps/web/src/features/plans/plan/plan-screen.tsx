import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, plans, toDate } from "@tokuly/api";
import {
  forgetPlan,
  formFromPlan,
  formProblem,
  isHostLike,
  type PlanFormValue,
  planTimeText,
  toUpdateRequest,
  untilText,
} from "@tokuly/core/plans";
import {
  BellRing,
  CalendarClock,
  CalendarPlus,
  DoorOpen,
  Flag,
  MapPin,
  MoreHorizontal,
  Pencil,
  Send,
  XCircle,
} from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { ShareToChat } from "#/features/chat";
import { CommentThread } from "#/features/comments";
import { ReportDialog } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { nameOf, planClient, rememberPlan, usePlan } from "../api";
import { formatDay, planTimeZone, ymd } from "../dates";
import { buildIcs, downloadIcs } from "../ics";
import { Checklist } from "./checklist";
import { planIcsEvent } from "./model";
import { Participants } from "./participants";
import { PlanForm } from "./plan-form";
import { RsvpCard } from "./rsvp";
import { CostSplit } from "./split";

// The map library is big: it loads when a plan with a place on the map is opened.
const StaticMapPreview = lazy(() =>
  import("#/features/map").then((module) => ({ default: module.StaticMapPreview })),
);

/** `/plans/<id>`: a plan with everything around it — answers, people, the checklist, the cost split, comments. */
export function PlanScreen({ planId }: { planId: string }) {
  const { t } = useTranslation("plans");
  const { data: plan, isPending, isError } = usePlan(planId);

  if (isPending) {
    return (
      <Page title={t("plan.title")} back="/plans" wide>
        <Skeleton className="h-52 rounded-card" />
        <Skeleton className="mt-4 h-32 rounded-card" />
      </Page>
    );
  }
  if (isError || !plan) {
    return (
      <Page title={t("plan.title")} back="/plans">
        <EmptyState
          emoji="🫥"
          title={t("plan.notFound")}
          body={t("plan.notFoundBody")}
          action={
            <Link
              to="/plans"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("plan.backToPlans")}
            </Link>
          }
        />
      </Page>
    );
  }
  return <LoadedPlan plan={plan} />;
}

function LoadedPlan({ plan }: { plan: plans.Plan }) {
  const { t, i18n } = useTranslation("plans");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<{ before: PlanFormValue; value: PlanFormValue }>();
  const [cancelling, setCancelling] = useState(false);
  const [reason, setReason] = useState("");
  const [leaving, setLeaving] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [reporting, setReporting] = useState(false);

  const cancelled = plan.status === plans.PlanStatus.CANCELLED;
  const host = plan.participants.find(
    (participant) => participant.role === plans.PlanRole.HOST,
  )?.user;
  const iAmHost = plan.myRole === plans.PlanRole.HOST;
  const subject = { type: common.SubjectType.PLAN, id: plan.id };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));

  const update = useMutation({
    mutationFn: (edit: { before: PlanFormValue; value: PlanFormValue }) =>
      planClient().updatePlan(toUpdateRequest(plan.id, edit.before, edit.value, planTimeZone())),
    onSuccess: (response) => {
      rememberPlan(queryClient, response.plan);
      setEditing(undefined);
      toast.success(t("plan.saved"));
    },
    onError,
  });
  const cancel = useMutation({
    mutationFn: () => planClient().cancelPlan({ planId: plan.id, reason: reason.trim() }),
    onSuccess: (response) => {
      rememberPlan(queryClient, response.plan);
      setCancelling(false);
      toast(t("plan.cancelledToast"));
    },
    onError,
  });
  const leave = useMutation({
    mutationFn: () => planClient().leavePlan({ planId: plan.id }),
    onSuccess: () => {
      forgetPlan(queryClient, plan.id);
      toast(t("plan.left"));
      void navigate({ to: "/plans" });
    },
    onError: (error) => {
      setLeaving(false);
      onError(error);
    },
  });

  const addToCalendar = () =>
    downloadIcs(
      `${plan.title || "plan"}.ics`,
      buildIcs([planIcsEvent(plan, `${window.location.origin}/plans/${plan.id}`)]),
    );
  const hasDay = Boolean(plan.time?.start.case);
  const actions: MenuAction[] = [
    ...(isHostLike(plan) && !cancelled
      ? [
          {
            label: t("plan.menu.edit"),
            icon: <Pencil />,
            onSelect: () => {
              const form = formFromPlan(plan, planTimeZone());
              setEditing({ before: form, value: form });
            },
          },
        ]
      : []),
    ...(hasDay && !cancelled
      ? [{ label: t("plan.menu.calendar"), icon: <CalendarPlus />, onSelect: addToCalendar }]
      : []),
    { label: t("plan.menu.share"), icon: <Send />, onSelect: () => setSharing(true) },
    ...(iAmHost
      ? []
      : [{ label: t("plan.menu.report"), icon: <Flag />, onSelect: () => setReporting(true) }]),
    ...(iAmHost
      ? []
      : [
          {
            label: t("plan.menu.leave"),
            icon: <DoorOpen />,
            danger: true,
            onSelect: () => setLeaving(true),
          },
        ]),
    ...(iAmHost && !cancelled
      ? [
          {
            label: t("plan.menu.cancel"),
            icon: <XCircle />,
            danger: true,
            onSelect: () => setCancelling(true),
          },
        ]
      : []),
  ];
  const until = !cancelled && !plan.over ? untilText(i18n, plan.time, planTimeZone()) : undefined;
  const cancelledAt = toDate(plan.cancelledAt);
  const editProblem = editing ? formProblem(editing.value, planTimeZone()) : null;

  return (
    <Page
      title={plan.title}
      back="/plans"
      wide
      actions={
        <Menu
          actions={actions}
          trigger={
            <Button variant="ghost" size="icon" aria-label={t("plan.menu.label")}>
              <MoreHorizontal className="size-6" />
            </Button>
          }
        />
      }
    >
      <div className="grid items-start gap-6 lg:grid-cols-2">
        <div className="flex flex-col gap-6">
          <Card className="animate-pop-in">
            <div className="flex items-start gap-4">
              <span
                className="grid size-20 shrink-0 place-items-center rounded-[1.4rem] border-[2.5px] border-ink bg-brand-soft text-5xl"
                aria-hidden
              >
                {plan.emoji || "📅"}
              </span>
              <div className="min-w-0 flex-1">
                <h2 className="text-2xl leading-tight font-black break-words">{plan.title}</h2>
                <div className="mt-1.5 flex flex-wrap gap-1.5">
                  {cancelled && <Chip className="bg-danger text-white">{t("plan.cancelled")}</Chip>}
                  {!cancelled && plan.over && <Chip>{t("plan.over")}</Chip>}
                  {until && <Chip className="bg-lime text-[#17122b]">{until}</Chip>}
                  {plan.pollId && (
                    <Link to="/plans/polls/$pollId" params={{ pollId: plan.pollId }}>
                      <Chip className="bg-sky text-[#17122b]">{t("plan.fromPoll")}</Chip>
                    </Link>
                  )}
                </div>
              </div>
            </div>

            {cancelled && (
              <p className="mt-4 rounded-2xl bg-danger/10 px-4 py-3 text-sm font-bold">
                {t("plan.cancelledBanner", {
                  date: cancelledAt ? formatDay(ymd(cancelledAt), i18n.language) : "",
                })}
                {plan.cancelReason && (
                  <span className="mt-1 block font-normal">「{plan.cancelReason}」</span>
                )}
              </p>
            )}

            <dl className="mt-4 flex flex-col gap-2.5">
              <div className="flex items-start gap-3">
                <dt className="grid size-9 shrink-0 place-items-center rounded-xl bg-sun text-[#17122b]">
                  <CalendarClock className="size-5" aria-label={t("plan.when")} />
                </dt>
                <dd className="min-w-0 flex-1 self-center font-bold" data-testid="plan-time">
                  {planTimeText(i18n, plan.time, planTimeZone())}
                </dd>
              </div>
              {plan.place?.name && (
                <div className="flex items-start gap-3">
                  <dt className="grid size-9 shrink-0 place-items-center rounded-xl bg-sky text-[#17122b]">
                    <MapPin className="size-5" aria-label={t("plan.where")} />
                  </dt>
                  <dd className="min-w-0 flex-1 self-center">
                    <span className="block font-bold break-words">{plan.place.name}</span>
                    {plan.place.address && (
                      <span className="block text-sm break-words text-ink-soft">
                        {plan.place.address}
                      </span>
                    )}
                    {plan.place.location && (
                      <Suspense fallback={<Skeleton className="mt-2 h-32" />}>
                        <StaticMapPreview
                          lat={plan.place.location.lat}
                          lng={plan.place.location.lng}
                          label={plan.place.name}
                          className="mt-2 aspect-auto h-32"
                        />
                      </Suspense>
                    )}
                  </dd>
                </div>
              )}
              {!cancelled &&
                !plan.over &&
                hasDay &&
                (plan.remindDayBefore || plan.remindMinutesBefore > 0) && (
                  <div className="flex items-start gap-3">
                    <dt className="grid size-9 shrink-0 place-items-center rounded-xl bg-pink text-white">
                      <BellRing className="size-5" aria-label={t("plan.reminders")} />
                    </dt>
                    <dd className="min-w-0 flex-1 self-center text-sm font-bold text-ink-soft">
                      {[
                        plan.remindDayBefore ? t("plan.remindDayBefore") : "",
                        plan.remindMinutesBefore > 0 && plan.time?.start.case === "startAt"
                          ? t("plan.remindMinutes", { count: plan.remindMinutesBefore })
                          : "",
                      ]
                        .filter(Boolean)
                        .join(" · ") || t("plan.remindDayBefore")}
                    </dd>
                  </div>
                )}
            </dl>
            {plan.description && (
              <p className="mt-4 leading-relaxed break-words whitespace-pre-wrap">
                {plan.description}
              </p>
            )}
            <p className="mt-4 text-sm font-bold text-ink-faint">
              {t("plan.hostedBy", { name: nameOf(host) })}
            </p>
          </Card>

          <RsvpCard plan={plan} />
          <Participants plan={plan} />
        </div>

        <div className="flex flex-col gap-6">
          <Checklist plan={plan} />
          <CostSplit plan={plan} />
          <CommentThread subject={subject} />
        </div>
      </div>

      <Dialog
        open={Boolean(editing)}
        onOpenChange={(open) => !open && setEditing(undefined)}
        title={t("plan.editTitle")}
        footer={
          <Button
            loading={update.isPending}
            disabled={Boolean(editProblem)}
            onClick={() => editing && update.mutate(editing)}
          >
            {t("plan.save")}
          </Button>
        }
      >
        {editing && (
          <PlanForm
            value={editing.value}
            onChange={(value) => setEditing({ ...editing, value })}
            problem={editProblem ? t(`plan.form.errors.${editProblem}`) : null}
          />
        )}
      </Dialog>
      <ConfirmDialog
        open={cancelling}
        onOpenChange={setCancelling}
        title={t("plan.cancelTitle")}
        description={t("plan.cancelBody")}
        confirmLabel={t("plan.cancelConfirm")}
        danger
        loading={cancel.isPending}
        onConfirm={() => cancel.mutate()}
      >
        <TextField
          label={t("plan.cancelReason")}
          placeholder={t("plan.cancelReasonPlaceholder")}
          value={reason}
          maxLength={200}
          onChange={(event) => setReason(event.target.value)}
        />
      </ConfirmDialog>
      <ConfirmDialog
        open={leaving}
        onOpenChange={setLeaving}
        title={t("plan.leaveTitle")}
        description={t("plan.leaveBody")}
        confirmLabel={t("plan.leaveConfirm")}
        danger
        loading={leave.isPending}
        onConfirm={() => leave.mutate()}
      />
      <ShareToChat
        open={sharing}
        onOpenChange={setSharing}
        share={{ url: `${window.location.origin}/plans/${plan.id}` }}
        title={plan.title}
      />
      <ReportDialog
        subject={subject}
        open={reporting}
        onOpenChange={setReporting}
        label={plan.title}
      />
    </Page>
  );
}
