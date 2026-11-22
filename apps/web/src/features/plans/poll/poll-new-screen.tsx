import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { type common, fromDate } from "@tokuly/api";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea, TextField } from "#/components/ui/field";
import { Page } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { errorMessage } from "#/lib/errors";
import { pollClient, rememberPoll } from "../api";
import { formatDay, fromDateTimeInput, planTimeZone } from "../dates";
import { FriendChips } from "../friend-picker";
import { ChoiceChips } from "../parts";
import { DatePicker } from "./date-picker";
import { buildSlots, MAX_SLOTS, type TimeRange } from "./model";

/** `/plans/polls/new`: a date poll — candidate days from a calendar, optionally with times. */
export function PollNewScreen() {
  const { t, i18n } = useTranslation("plans");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [days, setDays] = useState<string[]>([]);
  const [timed, setTimed] = useState(false);
  const [times, setTimes] = useState<TimeRange[]>([{ start: "18:00", end: "" }]);
  const [invitees, setInvitees] = useState<common.UserSummary[]>([]);
  const [openToFriends, setOpenToFriends] = useState(true);
  const [deadline, setDeadline] = useState("");
  const [tried, setTried] = useState(false);

  const slots = buildSlots(days, timed ? times : null);
  const problem = !title.trim()
    ? t("poll.form.errors.title")
    : slots.length === 0
      ? t("poll.form.errors.slots")
      : slots.length > MAX_SLOTS
        ? t("poll.form.errors.tooMany", { max: MAX_SLOTS })
        : null;

  // The deadline field is read on the account's clock.
  const deadlineAt = deadline ? fromDateTimeInput(deadline) : null;
  const create = useMutation({
    mutationFn: () =>
      pollClient().createPoll({
        title: title.trim(),
        description: description.trim(),
        slots,
        inviteeIds: invitees.map((user) => user.id),
        openToFriends,
        deadline: deadlineAt ? fromDate(deadlineAt) : undefined,
        timeZone: planTimeZone(),
      }),
    onSuccess: ({ poll }) => {
      rememberPoll(queryClient, poll);
      toast.success(t("poll.created"));
      if (poll)
        void navigate({
          to: "/plans/polls/$pollId",
          params: { pollId: poll.shortId || poll.id },
          replace: true,
        });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });

  return (
    <Page title={t("poll.newTitle")} back="/plans">
      <form
        className="flex flex-col gap-6"
        onSubmit={(event) => {
          event.preventDefault();
          setTried(true);
          if (!problem) create.mutate();
        }}
      >
        <TextField
          label={t("poll.form.title")}
          placeholder={t("poll.form.titlePlaceholder")}
          value={title}
          maxLength={80}
          onChange={(event) => setTitle(event.target.value)}
        />

        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("poll.form.days")}</p>
          <div className="sticker rounded-card bg-surface p-3">
            <DatePicker value={days} onChange={setDays} />
          </div>
          {days.length > 0 ? (
            <ul className="flex flex-wrap gap-1.5" aria-label={t("poll.form.chosen")}>
              {days.map((day) => (
                <li
                  key={day}
                  className="inline-flex h-8 items-center gap-1 rounded-full bg-lime pr-1 pl-3 text-sm font-black text-[#17122b]"
                >
                  {formatDay(day, i18n.language)}
                  <button
                    type="button"
                    aria-label={t("poll.form.removeDay", { day: formatDay(day, i18n.language) })}
                    onClick={() => setDays(days.filter((other) => other !== day))}
                    className="grid size-6 place-items-center rounded-full hover:bg-black/10"
                  >
                    <X className="size-3.5" />
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="px-1 text-sm text-ink-soft">{t("poll.form.daysHint")}</p>
          )}
        </div>

        <div className="flex flex-col gap-2">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("poll.form.timeMode")}</p>
          <ChoiceChips
            label={t("poll.form.timeMode")}
            value={timed ? "timed" : "day"}
            onChange={(mode) => setTimed(mode === "timed")}
            options={[
              { value: "day", label: t("poll.form.dayOnly") },
              { value: "timed", label: t("poll.form.withTime") },
            ]}
          />
          {timed && (
            <div className="flex flex-col gap-2 rounded-card bg-surface-2 p-3">
              {times.map((time, index) => (
                <div key={index} className="flex items-end gap-2">
                  <TextField
                    type="time"
                    className="min-w-0 flex-1"
                    label={index === 0 ? t("poll.form.from") : undefined}
                    aria-label={t("poll.form.from")}
                    value={time.start}
                    onChange={(event) =>
                      setTimes(
                        times.map((other, i) =>
                          i === index ? { ...other, start: event.target.value } : other,
                        ),
                      )
                    }
                  />
                  <TextField
                    type="time"
                    className="min-w-0 flex-1"
                    label={index === 0 ? t("poll.form.until") : undefined}
                    aria-label={t("poll.form.until")}
                    value={time.end}
                    onChange={(event) =>
                      setTimes(
                        times.map((other, i) =>
                          i === index ? { ...other, end: event.target.value } : other,
                        ),
                      )
                    }
                  />
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-12 shrink-0"
                    aria-label={t("poll.form.removeTime")}
                    disabled={times.length === 1}
                    onClick={() => setTimes(times.filter((_, i) => i !== index))}
                  >
                    <X className="size-5" />
                  </Button>
                </div>
              ))}
              <Button
                variant="ghost"
                size="sm"
                className="self-start"
                icon={<Plus className="size-4" />}
                onClick={() => setTimes([...times, { start: "", end: "" }])}
              >
                {t("poll.form.addTime")}
              </Button>
              <p className="px-1 text-xs text-ink-faint">{t("poll.form.timesHint")}</p>
            </div>
          )}
          <p className="px-1 text-sm font-bold" aria-live="polite">
            {t("poll.form.slotCount", { count: slots.length })}
          </p>
        </div>

        <FriendChips
          label={t("poll.form.invitees")}
          users={invitees}
          onChange={setInvitees}
          addLabel={t("poll.form.addInvitees")}
          pickerTitle={t("poll.form.inviteesTitle")}
        />
        <div className="rounded-card bg-surface p-1.5">
          <SwitchRow
            label={t("poll.form.openToFriends")}
            hint={t("poll.form.openToFriendsHint")}
            checked={openToFriends}
            onChange={setOpenToFriends}
          />
        </div>
        <TextField
          type="datetime-local"
          label={t("poll.form.deadline")}
          hint={t("poll.form.deadlineHint")}
          value={deadline}
          onChange={(event) => setDeadline(event.target.value)}
        />
        <TextArea
          label={t("poll.form.description")}
          placeholder={t("poll.form.descriptionPlaceholder")}
          value={description}
          maxLength={1000}
          onChange={(event) => setDescription(event.target.value)}
        />

        {tried && problem && (
          <p role="alert" className="px-1 text-sm font-bold text-danger">
            {problem}
          </p>
        )}
        <Button type="submit" size="lg" block loading={create.isPending}>
          {t("poll.create")}
        </Button>
      </form>
    </Page>
  );
}
