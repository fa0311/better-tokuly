import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { Check, HeartCrack, Sparkles, X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { anniversaryClient, keys, nameOf, useAnniversaries } from "../api";
import { formatDay, todayYmd } from "../dates";
import { ChoiceChips, SectionTitle } from "../parts";
import { anniversaryEmoji, anniversaryTitle, inDaysText, localized } from "./model";
import { markMomentSeen, momentSeen, TodayMoment } from "./today-moment";

/** `/plans/anniversaries/<id>`: a "days since" counter of two people, its milestones and its settings — or the invitation to one. */
export function AnniversaryScreen({ anniversaryId }: { anniversaryId: string }) {
  const { t } = useTranslation("plans");
  const anniversaries = useAnniversaries();
  const anniversary = anniversaries.data?.find((item) => item.id === anniversaryId);

  if (anniversaries.isPending) {
    return (
      <Page title={t("anniv.title")} back="/plans">
        <Skeleton className="h-72 rounded-card" />
      </Page>
    );
  }
  if (!anniversary) {
    return (
      <Page title={t("anniv.title")} back="/plans">
        <EmptyState
          emoji="🍂"
          title={t("anniv.notFound")}
          body={t("anniv.notFoundBody")}
          action={
            <Link to="/plans" search={{ tab: "moments" }} className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("plan.backToPlans")}
            </Link>
          }
        />
      </Page>
    );
  }
  return <LoadedAnniversary key={anniversary.id} anniversary={anniversary} />;
}

function LoadedAnniversary({ anniversary }: { anniversary: plans.Anniversary }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [ending, setEnding] = useState(false);
  const [celebrating, setCelebrating] = useState(false);
  const [title, setTitle] = useState<string | null>(null);
  const [startDate, setStartDate] = useState<string | null>(null);

  const couple = anniversary.kind === plans.AnniversaryKind.COUPLE;
  const pending = anniversary.status === plans.AnniversaryStatus.PENDING;
  const today = anniversary.upcoming.find((milestone) => milestone.daysLeft === 0);
  const todayLabel = localized(today?.label, i18n.language);
  useEffect(() => {
    if (today && !momentSeen(anniversary.id, today.date)) {
      markMomentSeen(anniversary.id, today.date);
      setCelebrating(true);
    }
  }, [today, anniversary.id]);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.anniversaries });
    void queryClient.invalidateQueries({ queryKey: keys.calendars });
    void queryClient.invalidateQueries({ queryKey: ["identity", "profile"] });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));
  const update = useMutation({
    mutationFn: (change: { startDate?: string; countStartDay?: boolean; title?: string; emoji?: string; showOnProfile?: boolean }) => anniversaryClient().updateAnniversary({ anniversaryId: anniversary.id, ...change }),
    onSuccess: (response) => {
      queryClient.setQueryData<plans.Anniversary[]>(keys.anniversaries, (list) => list?.map((item) => (item.id === anniversary.id && response.anniversary ? response.anniversary : item)));
      refresh();
      setTitle(null);
      setStartDate(null);
    },
    onError,
  });
  const respond = useMutation({
    mutationFn: (accept: boolean) => anniversaryClient().respondCoupleInvite({ anniversaryId: anniversary.id, accept }),
    onSuccess: (_response, accept) => {
      refresh();
      if (accept) toast.success(t("anniv.accepted"));
      else void navigate({ to: "/plans", search: { tab: "moments" } });
    },
    onError,
  });
  const end = useMutation({
    mutationFn: () => anniversaryClient().endAnniversary({ anniversaryId: anniversary.id }),
    onSuccess: () => {
      queryClient.setQueryData<plans.Anniversary[]>(keys.anniversaries, (list) => list?.filter((item) => item.id !== anniversary.id));
      refresh();
      toast(pending ? t("anniv.withdrawn") : t("anniv.ended"));
      void navigate({ to: "/plans", search: { tab: "moments" } });
    },
    onError,
  });

  const name = nameOf(anniversary.partner);
  const titleText = title ?? anniversary.title;
  const startText = startDate ?? anniversary.startDate;

  return (
    <Page title={anniversaryTitle(anniversary, t)} back="/plans">
      <div className={cn("sticker-lg flex animate-pop-in flex-col items-center rounded-[2rem] px-6 py-9 text-center", pending ? "bg-surface" : couple ? "bg-pink text-white" : "bg-sky text-[#17122b]")} data-testid="anniversary-hero">
        <div className="flex items-center">
          <Avatar user={me?.user} size="lg" outlined className="z-10" />
          <span className="-mx-2 z-20 text-4xl" aria-hidden>
            {anniversaryEmoji(anniversary)}
          </span>
          <Avatar user={anniversary.partner} size="lg" outlined />
        </div>
        <h2 className="mt-4 text-xl font-black">{t("anniv.with", { name })}</h2>
        {pending ? (
          <p className="mt-3 max-w-xs font-bold text-ink-soft">{anniversary.invitedByMe ? t("anniv.waitingBody", { name }) : t("anniv.inviteBody", { name })}</p>
        ) : (
          <>
            <p className="mt-3 flex items-baseline gap-2 font-black">
              <span className="text-8xl leading-none tabular-nums">{anniversary.days}</span>
              <span className="text-xl">{anniversary.countStartDay ? t("anniv.dayNth") : t("anniv.daysUnit")}</span>
            </p>
            <p className="mt-3 font-bold opacity-80">{t("anniv.since", { date: formatDay(anniversary.startDate, i18n.language) })}</p>
            {today && (
              <Button variant="secondary" className="mt-5" icon={<Sparkles className="size-4" />} onClick={() => setCelebrating(true)}>
                {todayLabel}
              </Button>
            )}
          </>
        )}
      </div>

      {pending ? (
        <div className="mt-6 flex flex-col gap-2.5">
          {!anniversary.invitedByMe && (
            <Button variant="lime" size="lg" block icon={<Check className="size-5" />} loading={respond.isPending && respond.variables === true} onClick={() => respond.mutate(true)}>
              {t("anniv.accept")}
            </Button>
          )}
          {anniversary.invitedByMe ? (
            <Button variant="secondary" block icon={<X className="size-4" />} loading={end.isPending} onClick={() => end.mutate()}>
              {t("anniv.withdraw")}
            </Button>
          ) : (
            <Button variant="ghost" block loading={respond.isPending && respond.variables === false} onClick={() => respond.mutate(false)}>
              {t("anniv.decline")}
            </Button>
          )}
        </div>
      ) : (
        <>
          {anniversary.upcoming.length > 0 && (
            <section className="mt-6">
              <SectionTitle>{t("anniv.milestones")}</SectionTitle>
              <ul className="rounded-card bg-surface p-1.5">
                {anniversary.upcoming.map((milestone) => (
                  <li key={`${milestone.kind}-${milestone.number}`} className="flex items-center gap-3 rounded-2xl px-3 py-2.5" data-testid="milestone">
                    <span className="text-2xl" aria-hidden>
                      {milestone.kind === plans.MilestoneKind.DAYS ? "💯" : milestone.kind === plans.MilestoneKind.HALF_YEAR ? "🌗" : "🎂"}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-black">{localized(milestone.label, i18n.language)}</span>
                      <span className="block text-sm text-ink-soft">{formatDay(milestone.date, i18n.language)}</span>
                    </span>
                    <span className={cn("shrink-0 rounded-full px-3 py-1 text-sm font-black", milestone.daysLeft === 0 ? "bg-lime text-[#17122b]" : "bg-surface-2 text-ink-soft")}>
                      {inDaysText(milestone.daysLeft, t)}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          )}

          <section className="mt-6">
            <SectionTitle>{t("anniv.settings")}</SectionTitle>
            <div className="flex flex-col gap-3 rounded-card bg-surface p-4">
              <form
                className="flex items-end gap-2"
                onSubmit={(event) => {
                  event.preventDefault();
                  update.mutate({ title: titleText.trim() }, { onSuccess: () => toast.success(t("anniv.saved")) });
                }}
              >
                <EmojiPicker
                  onPick={(emoji) => update.mutate({ emoji })}
                  trigger={
                    <button type="button" aria-label={t("plan.form.emoji")} className="pressable sticker grid size-12 shrink-0 place-items-center rounded-2xl bg-brand-soft text-2xl">
                      {anniversaryEmoji(anniversary)}
                    </button>
                  }
                />
                <TextField className="min-w-0 flex-1" label={t("anniv.name")} placeholder={anniversaryTitle({ ...anniversary, title: "" }, t)} value={titleText} maxLength={40} onChange={(event) => setTitle(event.target.value)} />
                {title !== null && title.trim() !== anniversary.title && (
                  <Button type="submit" variant="secondary" className="h-12 shrink-0" loading={update.isPending}>
                    {t("plan.save")}
                  </Button>
                )}
              </form>
              <div className="flex items-end gap-2">
                <TextField type="date" className="min-w-0 flex-1" label={t("anniv.startDate")} value={startText} max={todayYmd()} onChange={(event) => setStartDate(event.target.value)} />
                {startDate !== null && startDate !== anniversary.startDate && startDate && (
                  <Button variant="secondary" className="h-12 shrink-0" loading={update.isPending} onClick={() => update.mutate({ startDate }, { onSuccess: () => toast.success(t("anniv.saved")) })}>
                    {t("plan.save")}
                  </Button>
                )}
              </div>
              <div className="flex flex-col gap-2">
                <p className="px-1 text-sm font-bold text-ink-soft">{t("anniv.countStyle")}</p>
                <ChoiceChips
                  label={t("anniv.countStyle")}
                  value={anniversary.countStartDay ? "nth" : "since"}
                  onChange={(style) => update.mutate({ countStartDay: style === "nth" })}
                  options={[
                    { value: "nth", label: t("anniv.countNth") },
                    { value: "since", label: t("anniv.countSince") },
                  ]}
                />
              </div>
              {couple && (
                <SwitchRow
                  label={t("anniv.showOnProfile")}
                  hint={anniversary.showOnProfile && !anniversary.partnerShowsOnProfile ? t("anniv.showOnProfileWaiting", { name }) : t("anniv.showOnProfileHint")}
                  checked={anniversary.showOnProfile}
                  disabled={update.isPending}
                  onChange={(showOnProfile) => update.mutate({ showOnProfile })}
                />
              )}
              <p className="px-1 text-xs text-ink-faint">{t("anniv.sharedHint", { name })}</p>
            </div>
          </section>

          <Button variant="ghost" block className="mt-6 text-danger" icon={<HeartCrack className="size-4" />} onClick={() => setEnding(true)}>
            {t("anniv.end")}
          </Button>
        </>
      )}

      <ConfirmDialog open={ending} onOpenChange={setEnding} title={t("anniv.endTitle")} description={t("anniv.endBody", { name })} confirmLabel={t("anniv.end")} danger loading={end.isPending} onConfirm={() => end.mutate()} />
      <TodayMoment open={celebrating} onOpenChange={setCelebrating} emoji={anniversaryEmoji(anniversary)} kicker={t("anniv.momentKicker", { name })} title={todayLabel} detail={t("anniv.momentDetail", { days: anniversary.days })} />
    </Page>
  );
}
