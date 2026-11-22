import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import { ChevronRight, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { anniversaryClient, keys, nameOf, useAnniversaries, useBirthdays, useCountdowns } from "../api";
import { todayYmd } from "../dates";
import { FriendPicker } from "../friend-picker";
import { ChoiceChips, SectionTitle } from "../parts";
import { AnniversaryCard, CountdownCard } from "./countdown-card";
import { CountdownDialog } from "./countdown-dialog";
import { inDaysText } from "./model";

/** The "カウントダウン" tab: countdowns, "days since" counters and friends' birthdays. */
export function Moments() {
  const { t } = useTranslation("plans");
  const countdowns = useCountdowns();
  const anniversaries = useAnniversaries();
  const [creating, setCreating] = useState(false);
  const [starting, setStarting] = useState(false);

  return (
    <div className="flex flex-col gap-8">
      <section>
        <SectionTitle
          action={
            <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
              {t("countdown.add")}
            </Button>
          }
        >
          {t("countdown.listTitle")}
        </SectionTitle>
        {countdowns.isPending ? (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            <Skeleton className="h-36 rounded-card" />
            <Skeleton className="h-36 rounded-card" />
          </div>
        ) : countdowns.data?.length ? (
          <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
            {countdowns.data.map((countdown) => (
              <li key={countdown.id}>
                <CountdownCard countdown={countdown} />
              </li>
            ))}
          </ul>
        ) : (
          <div className="rounded-card bg-surface">
            <EmptyState emoji="⏳" title={t("countdown.empty")} body={t("countdown.emptyBody")} />
          </div>
        )}
      </section>

      <section>
        <SectionTitle
          action={
            <Button variant="secondary" size="sm" icon={<Plus className="size-4" />} onClick={() => setStarting(true)}>
              {t("anniv.add")}
            </Button>
          }
        >
          {t("anniv.listTitle")}
        </SectionTitle>
        {anniversaries.isPending ? (
          <Skeleton className="h-24 rounded-card" />
        ) : anniversaries.data?.length ? (
          <ul className="grid gap-3 lg:grid-cols-2">
            {anniversaries.data.map((anniversary) => (
              <li key={anniversary.id}>
                <AnniversaryCard anniversary={anniversary} />
              </li>
            ))}
          </ul>
        ) : (
          <p className="rounded-card border-2 border-dashed border-line px-4 py-6 text-center text-sm font-bold text-ink-soft">{t("anniv.empty")}</p>
        )}
      </section>

      <Birthdays />

      <CountdownDialog open={creating} onOpenChange={setCreating} />
      <StartCounterDialog open={starting} onOpenChange={setStarting} />
    </div>
  );
}

function Birthdays() {
  const { t, i18n } = useTranslation("plans");
  const queryClient = useQueryClient();
  const birthdays = useBirthdays();
  const settings = useQuery({ queryKey: keys.settings, queryFn: async () => (await anniversaryClient().getPlansSettings({})).settings });
  const share = useMutation({
    mutationFn: (shareBirthday: boolean) => anniversaryClient().updatePlansSettings({ shareBirthday }),
    onMutate: (shareBirthday) => queryClient.setQueryData(keys.settings, (current: unknown) => ({ ...(current as object), shareBirthday })),
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: "plans" }));
      void queryClient.invalidateQueries({ queryKey: keys.settings });
    },
  });
  const dayText = (month: number, day: number) => new Intl.DateTimeFormat(i18n.language, { month: "long", day: "numeric" }).format(new Date(2024, month - 1, day));

  return (
    <section>
      <SectionTitle>{t("birthday.title")}</SectionTitle>
      <div className="rounded-card bg-surface p-1.5">
        {birthdays.isPending ? (
          <Skeleton className="m-1.5 h-12" />
        ) : birthdays.data?.length ? (
          <ul>
            {birthdays.data.map((birthday) => (
              <li key={birthday.user?.id}>
                <Link to="/u/$handle" params={{ handle: birthday.user?.handle ?? "" }} className="flex items-center gap-3 rounded-2xl px-3 py-2.5 hover:bg-surface-2">
                  <Avatar user={birthday.user} size="sm" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-bold">{nameOf(birthday.user)}</span>
                    <span className="block text-sm text-ink-soft">🎂 {dayText(birthday.month, birthday.day)}</span>
                  </span>
                  <span className={birthday.daysLeft === 0 ? "rounded-full bg-lime px-3 py-1 text-sm font-black text-[#17122b]" : "text-sm font-bold text-ink-soft"}>
                    {birthday.daysLeft === 0 ? t("birthday.today") : inDaysText(birthday.daysLeft, t)}
                  </span>
                  <ChevronRight className="size-4 shrink-0 text-ink-faint" />
                </Link>
              </li>
            ))}
          </ul>
        ) : (
          <p className="px-3 py-4 text-sm font-bold text-ink-soft">{t("birthday.empty")}</p>
        )}
        <div className="mt-1 border-t-2 border-line pt-1">
          <SwitchRow label={t("birthday.share")} hint={t("birthday.shareHint")} checked={settings.data?.shareBirthday ?? false} disabled={settings.isPending} onChange={(checked) => share.mutate(checked)} />
        </div>
      </div>
    </section>
  );
}

/** Starts a "days since" counter with a friend, or invites one to a couple counter. */
function StartCounterDialog({ open, onOpenChange }: { open: boolean; onOpenChange(open: boolean): void }) {
  const { t } = useTranslation("plans");
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<"friendship" | "couple">("friendship");
  const [friend, setFriend] = useState<common.UserSummary>();
  const [startDate, setStartDate] = useState("");
  const [picking, setPicking] = useState(false);
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setKind("friendship");
      setFriend(undefined);
      setStartDate("");
    }
  }

  const start = useMutation({
    mutationFn: async () => {
      if (kind === "couple") await anniversaryClient().inviteCouple({ userId: friend?.id ?? "", startDate });
      else await anniversaryClient().startFriendshipCounter({ userId: friend?.id ?? "", startDate });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.anniversaries });
      void queryClient.invalidateQueries({ queryKey: keys.calendars });
      onOpenChange(false);
      toast.success(kind === "couple" ? t("anniv.invited", { name: nameOf(friend) }) : t("anniv.started"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={onOpenChange}
        title={t("anniv.newTitle")}
        description={t("anniv.newLead")}
        footer={
          <Button loading={start.isPending} disabled={!friend} onClick={() => start.mutate()}>
            {kind === "couple" ? t("anniv.sendInvite") : t("anniv.start")}
          </Button>
        }
      >
        <div className="flex flex-col gap-4">
          <ChoiceChips<"friendship" | "couple">
            label={t("anniv.kind")}
            value={kind}
            onChange={setKind}
            options={[
              { value: "friendship", label: `🤝 ${t("anniv.friendship")}` },
              { value: "couple", label: `💞 ${t("anniv.couple")}` },
            ]}
          />
          <p className="-mt-2 px-1 text-sm text-ink-soft">{kind === "couple" ? t("anniv.coupleHint") : t("anniv.friendshipHint")}</p>
          <button type="button" onClick={() => setPicking(true)} className="flex items-center gap-3 rounded-2xl border-2 border-line bg-surface px-3 py-2.5 text-left hover:border-ink-faint">
            {friend ? <Avatar user={friend} size="sm" /> : <span className="grid size-8 place-items-center rounded-[0.6rem] border-2 border-dashed border-ink-faint text-ink-faint">?</span>}
            <span className="min-w-0 flex-1 truncate font-bold">{friend ? nameOf(friend) : t("anniv.pickFriend")}</span>
            <ChevronRight className="size-4 text-ink-faint" />
          </button>
          <TextField type="date" label={t("anniv.startDate")} hint={kind === "couple" ? t("anniv.startHintCouple") : t("anniv.startHintFriend")} value={startDate} max={todayYmd()} onChange={(event) => setStartDate(event.target.value)} />
        </div>
      </Dialog>
      <FriendPicker
        open={picking}
        onOpenChange={setPicking}
        title={t("anniv.pickFriend")}
        single
        onConfirm={(users) => {
          setFriend(users[0]);
          setPicking(false);
        }}
      />
    </>
  );
}
