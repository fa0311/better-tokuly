import { useNavigate } from "@tanstack/react-router";
import type { plans } from "@tokuly/api";
import { BellRing, CalendarPlus, MapPin, MessageCircle, Send } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { openDirectChat } from "#/features/chat";
import { nameOf } from "../api";
import { type PlanDraft, setPlanDraft } from "../draft";
import { useNow } from "../parts";
import { useAnswerFree, useFreeAlerts } from "./answer";
import { draftFromStatus, expiryText, whenText } from "./model";

/**
 * A friend's status up close: what they wrote, until when, and what to do
 * about it — "あそぼ!" with a few words, a chat, or a plan for the two.
 */
export function FreeFriendSheet({
  status,
  draft,
  onOpenChange,
}: {
  /** The status shown; the sheet is closed while this is unset. */
  status: plans.FreeStatus | undefined;
  /** The plan the server prepared when "あそぼ!" was answered a moment ago. */
  draft?: PlanDraft;
  onOpenChange(open: boolean): void;
}) {
  const { t, i18n } = useTranslation("plans");
  const navigate = useNavigate();
  const now = useNow();
  const answer = useAnswerFree();
  const alerts = useFreeAlerts();
  const [message, setMessage] = useState("");
  const [serverDraft, setServerDraft] = useState<PlanDraft | undefined>();
  // Kept while the sheet slides away, so its content does not vanish first.
  const [last, setLast] = useState(status);
  if (status && status !== last) {
    setLast(status);
    if (status.user?.id !== last?.user?.id) {
      setMessage("");
      setServerDraft(undefined);
    }
  }
  const shown = status ?? last;
  const user = shown?.user;
  if (!shown || !user) return null;

  const answered = shown.answeredByMe;
  const alertOn = alerts.users.some((other) => other.id === user.id);
  const send = () =>
    answer.mutate(
      { status: shown, message },
      {
        onSuccess: (response) => {
          setServerDraft(response.planDraft);
          toast.success(t("free.sent", { name: nameOf(user) }));
        },
      },
    );
  const startPlan = () => {
    setPlanDraft(serverDraft ?? draft ?? draftFromStatus(shown, now));
    onOpenChange(false);
    void navigate({ to: "/plans/new" });
  };

  return (
    <Dialog open={Boolean(status)} onOpenChange={onOpenChange} title={t("free.friend.title", { name: nameOf(user) })}>
      <div className="flex flex-col gap-4">
        <div className="flex items-center gap-4">
          <Avatar user={user} size="lg" outlined />
          <div className="min-w-0 flex-1">
            <p className="text-lg font-black">{whenText(shown, now, i18n.language, t)}</p>
            <p className="text-sm font-bold text-ink-soft">⏳ {expiryText(shown, now, t)}</p>
            {shown.areaName && (
              <p className="mt-0.5 flex items-center gap-1 truncate text-sm font-bold text-ink-soft">
                <MapPin className="size-4 shrink-0" />
                {shown.areaName}
              </p>
            )}
          </div>
        </div>
        {shown.note && <p className="sticker-sm rounded-2xl bg-sun px-4 py-3 font-bold text-[#17122b]">「{shown.note}」</p>}

        <form
          className="flex items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            send();
          }}
        >
          <TextField
            className="min-w-0 flex-1"
            aria-label={t("free.friend.message")}
            placeholder={t("free.friend.messagePlaceholder")}
            value={message}
            maxLength={100}
            onChange={(event) => setMessage(event.target.value)}
          />
          <Button type="submit" variant="lime" className="h-12 shrink-0" loading={answer.isPending} icon={<Send className="size-4" />}>
            {answered ? t("free.friend.again") : t("free.asobo")}
          </Button>
        </form>
        {answered && <p className="-mt-2 px-1 text-sm font-bold text-success">✓ {t("free.friend.answered")}</p>}

        <div className="grid grid-cols-2 gap-2.5">
          <Button
            variant="secondary"
            icon={<MessageCircle className="size-4" />}
            onClick={() => {
              onOpenChange(false);
              void openDirectChat(user.id);
            }}
          >
            {t("free.friend.chat")}
          </Button>
          <Button variant="primary" icon={<CalendarPlus className="size-4" />} onClick={startPlan}>
            {t("free.friend.plan")}
          </Button>
        </div>

        <SwitchRow
          label={
            <span className="flex items-center gap-1.5">
              <BellRing className="size-4" />
              {t("free.alerts.toggle")}
            </span>
          }
          hint={t("free.alerts.toggleHint", { name: nameOf(user) })}
          checked={alertOn}
          disabled={alerts.isPending || alerts.set.isPending}
          onChange={(enabled) => alerts.set.mutate({ userIds: [user.id], enabled })}
        />
      </div>
    </Dialog>
  );
}
