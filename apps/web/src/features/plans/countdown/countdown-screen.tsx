import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { plans } from "@tokuly/api";
import { profileKeys } from "@tokuly/core/profile";
import { Pencil, Pin, PinOff, Repeat, Sparkles, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { anniversaryClient, keys, nameOf } from "../api";
import { formatDay } from "../dates";
import { SectionTitle } from "../parts";
import { CountdownDialog } from "./countdown-dialog";
import { countdownParts, tintOf } from "./model";
import { markMomentSeen, momentSeen, TodayMoment } from "./today-moment";

/** `/plans/countdowns/<id>`: one countdown, big; on its day the whole screen celebrates. */
export function CountdownScreen({ countdownId }: { countdownId: string }) {
  const { t, i18n } = useTranslation("plans");
  const me = useMe();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const {
    data: countdown,
    isPending,
    isError,
  } = useQuery({
    queryKey: keys.countdown(countdownId),
    queryFn: async () =>
      (await anniversaryClient().getCountdown({ countdownId })).countdown ?? null,
  });
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [celebrating, setCelebrating] = useState(false);
  const myId = me?.user?.id ?? "";

  // On the day, the moment opens by itself the first time the countdown is looked at.
  const isToday = countdown?.daysLeft === 0;
  const day = countdown?.nextDate || countdown?.date || "";
  useEffect(() => {
    if (isToday && myId && !momentSeen(myId, countdownId, day)) {
      markMomentSeen(myId, countdownId, day);
      setCelebrating(true);
    }
  }, [isToday, myId, countdownId, day]);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.countdowns });
    void queryClient.invalidateQueries({ queryKey: keys.calendars });
    void queryClient.invalidateQueries({ queryKey: profileKeys.profiles });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "plans" }));
  const pin = useMutation({
    // Unpinning goes back to private: the people it was shared with before are not remembered.
    mutationFn: (pinned: boolean) =>
      anniversaryClient().updateCountdown({
        countdownId,
        sharing: pinned ? plans.CountdownSharing.PROFILE : plans.CountdownSharing.PRIVATE,
        sharedWithIds: [],
      }),
    onSuccess: (response, pinned) => {
      queryClient.setQueryData(keys.countdown(countdownId), response.countdown ?? null);
      refresh();
      toast.success(pinned ? t("countdown.pinnedToast") : t("countdown.unpinnedToast"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => anniversaryClient().deleteCountdown({ countdownId }),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: keys.countdown(countdownId) });
      refresh();
      toast(t("countdown.deleted"));
      void navigate({ to: "/plans", search: { tab: "moments" } });
    },
    onError,
  });

  if (isPending) {
    return (
      <Page title={t("countdown.title")} back="/plans">
        <Skeleton className="h-72 rounded-card" />
      </Page>
    );
  }
  if (isError || !countdown) {
    return (
      <Page title={t("countdown.title")} back="/plans">
        <EmptyState
          emoji="⌛"
          title={t("countdown.notFound")}
          body={t("countdown.notFoundBody")}
          action={
            <Link
              to="/plans"
              search={{ tab: "moments" }}
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("plan.backToPlans")}
            </Link>
          }
        />
      </Page>
    );
  }

  const mine = countdown.owner?.id === me?.user?.id;
  const pinned = countdown.sharing === plans.CountdownSharing.PROFILE;
  const parts = countdownParts(countdown.daysLeft, t);
  return (
    <Page title={countdown.title} back="/plans">
      <div
        className={cn(
          "sticker-lg flex animate-pop-in flex-col items-center rounded-[2rem] px-6 py-10 text-center",
          countdown.daysLeft < 0 ? "bg-surface-2 text-ink-soft" : tintOf(countdown.id),
        )}
        data-testid="countdown-hero"
      >
        <span className={cn("text-7xl", isToday && "animate-float")} aria-hidden>
          {countdown.emoji || "⏳"}
        </span>
        <h2 className="mt-4 text-2xl font-black break-words">{countdown.title}</h2>
        <p className="mt-4 flex items-baseline gap-2 font-black">
          {parts.lead && <span className="text-xl">{parts.lead}</span>}
          <span className={cn("leading-none tabular-nums", isToday ? "text-6xl" : "text-8xl")}>
            {parts.number}
          </span>
          {parts.unit && <span className="text-xl">{parts.unit}</span>}
        </p>
        <p className="mt-4 flex items-center gap-1.5 font-bold opacity-80">
          {formatDay(countdown.nextDate || countdown.date, i18n.language)}
          {countdown.repeatsYearly && (
            <>
              <Repeat className="size-4" />
              {t("countdown.yearly")}
            </>
          )}
        </p>
        {isToday && (
          <Button
            variant="secondary"
            className="mt-6"
            icon={<Sparkles className="size-4" />}
            onClick={() => setCelebrating(true)}
          >
            {t("countdown.celebrate")}
          </Button>
        )}
      </div>

      {!mine && (
        <p className="mt-5 flex items-center justify-center gap-2 text-sm font-bold text-ink-soft">
          <Avatar user={countdown.owner} size="xs" />
          {t("countdown.sharedBy", { name: nameOf(countdown.owner) })}
        </p>
      )}

      {mine && (
        <>
          <div className="mt-6 grid grid-cols-2 gap-2.5 sm:grid-cols-3">
            <Button
              variant={pinned ? "secondary" : "lime"}
              className="col-span-2 sm:col-span-1"
              icon={pinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
              loading={pin.isPending}
              disabled={countdown.daysLeft < 0}
              onClick={() => pin.mutate(!pinned)}
            >
              {pinned ? t("countdown.unpin") : t("countdown.pin")}
            </Button>
            <Button
              variant="secondary"
              icon={<Pencil className="size-4" />}
              onClick={() => setEditing(true)}
            >
              {t("countdown.edit")}
            </Button>
            <Button
              variant="ghost"
              className="text-danger"
              icon={<Trash2 className="size-4" />}
              onClick={() => setDeleting(true)}
            >
              {t("countdown.delete")}
            </Button>
          </div>
          <section className="mt-6">
            <SectionTitle>{t("countdown.whoSees")}</SectionTitle>
            <div className="rounded-card bg-surface p-4 text-sm font-bold">
              {pinned ? (
                <p>📌 {t("countdown.seenProfile")}</p>
              ) : countdown.sharing === plans.CountdownSharing.CHOSEN ? (
                <>
                  <p>{t("countdown.seenChosen", { count: countdown.sharedWith.length })}</p>
                  <ul className="mt-2 flex flex-wrap gap-2">
                    {countdown.sharedWith.map((user) => (
                      <li
                        key={user.id}
                        className="inline-flex items-center gap-1.5 rounded-full bg-surface-2 py-1 pr-3 pl-1"
                      >
                        <Avatar user={user} size="xs" />
                        <EmojiText text={nameOf(user)} />
                      </li>
                    ))}
                  </ul>
                </>
              ) : (
                <p>🔒 {t("countdown.seenPrivate")}</p>
              )}
            </div>
          </section>
        </>
      )}

      <CountdownDialog open={editing} onOpenChange={setEditing} countdown={countdown} />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("countdown.deleteTitle")}
        description={t("countdown.deleteBody")}
        confirmLabel={t("countdown.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <TodayMoment
        open={celebrating}
        onOpenChange={setCelebrating}
        emoji={countdown.emoji}
        kicker={t("countdown.momentKicker")}
        title={countdown.title}
        detail={formatDay(day, i18n.language)}
      />
    </Page>
  );
}
