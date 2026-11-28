import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowRightLeft } from "lucide-react";
import { type FormEvent, type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { useAccountTimeZone } from "#/lib/account-time";
import { profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { botClient, type dev, keys } from "./api";
import { formatDateTime, Notice, Section } from "./parts";

const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "dev" }));

/** The open offers to hand a bot over, made by and to the caller. */
export function useBotTransfers() {
  return useQuery({
    queryKey: keys.botTransfers,
    queryFn: () => botClient().listBotTransfers({}),
  });
}

function useRefresh() {
  const queryClient = useQueryClient();
  return async () => {
    await queryClient.invalidateQueries({ queryKey: keys.botTransfers });
    await queryClient.invalidateQueries({ queryKey: keys.bots });
  };
}

/** Withdraws (the offerer) or declines (the receiver) an offer. */
function useCancel(done: string) {
  const { t } = useTranslation("dev");
  const refresh = useRefresh();
  return useMutation({
    mutationFn: (transferId: string) => botClient().cancelBotTransfer({ transferId }),
    onSuccess: async () => {
      toast.success(t(done));
      await refresh();
    },
    onError,
  });
}

/**
 * On the bots screen: bots offered to the caller (accept or decline) and
 * the caller's offers still waiting for an answer (withdraw). Nothing when
 * there are none.
 */
export function BotTransferInbox() {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const zone = useAccountTimeZone();
  const refresh = useRefresh();
  const transfers = useBotTransfers();
  const accept = useMutation({
    mutationFn: (transferId: string) => botClient().acceptBotTransfer({ transferId }),
    onSuccess: async (response) => {
      toast.success(t("bots.transfer.accepted"));
      await refresh();
      const botId = response.bot?.user?.id;
      if (botId) await navigate({ to: "/developers/bots/$botId", params: { botId } });
    },
    onError,
  });
  const decline = useCancel("bots.transfer.declined");
  const withdraw = useCancel("bots.transfer.withdrawn");
  const incoming = transfers.data?.incoming ?? [];
  const outgoing = transfers.data?.outgoing ?? [];
  if (incoming.length === 0 && outgoing.length === 0) return null;

  const row = (transfer: dev.BotTransfer, text: string, actions: ReactNode) => (
    <li
      key={transfer.id}
      className="flex flex-wrap items-center gap-3 p-3"
      data-testid="bot-transfer"
    >
      <Avatar user={transfer.bot} />
      <span className="min-w-0 flex-1 basis-40">
        <span className="block truncate font-bold">{text}</span>
        <span className="block text-xs text-ink-faint">
          {t("bots.transfer.until", { date: formatDateTime(transfer.expiresAt, zone) })}
        </span>
      </span>
      <span className="flex gap-2">{actions}</span>
    </li>
  );
  return (
    <>
      {incoming.length > 0 && (
        <Section title={t("bots.transfer.incoming")} className="mb-6">
          <Notice className="mb-3">{t("bots.transfer.inheritsAccept")}</Notice>
          <Card className="p-1">
            <ul className="divide-y-2 divide-line">
              {incoming.map((transfer) =>
                row(
                  transfer,
                  t("bots.transfer.incomingRow", {
                    from: transfer.from?.displayName ?? "",
                    bot: transfer.bot?.displayName ?? "",
                  }),
                  <>
                    <Button
                      size="sm"
                      loading={accept.isPending && accept.variables === transfer.id}
                      onClick={() => accept.mutate(transfer.id)}
                    >
                      {t("bots.transfer.accept")}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      loading={decline.isPending && decline.variables === transfer.id}
                      onClick={() => decline.mutate(transfer.id)}
                    >
                      {t("bots.transfer.decline")}
                    </Button>
                  </>,
                ),
              )}
            </ul>
          </Card>
        </Section>
      )}
      {outgoing.length > 0 && (
        <Section title={t("bots.transfer.outgoing")} className="mb-6">
          <Card className="p-1">
            <ul className="divide-y-2 divide-line">
              {outgoing.map((transfer) =>
                row(
                  transfer,
                  t("bots.transfer.outgoingRow", {
                    bot: transfer.bot?.displayName ?? "",
                    to: transfer.to?.displayName ?? "",
                  }),
                  <Button
                    size="sm"
                    variant="secondary"
                    loading={withdraw.isPending && withdraw.variables === transfer.id}
                    onClick={() => withdraw.mutate(transfer.id)}
                  >
                    {t("bots.transfer.withdraw")}
                  </Button>,
                ),
              )}
            </ul>
          </Card>
        </Section>
      )}
    </>
  );
}

/** On a bot's page: offer it to someone by their handle, or see (and withdraw) the open offer. */
export function HandOverSection({ bot }: { bot: dev.Bot }) {
  const { t } = useTranslation("dev");
  const refresh = useRefresh();
  const botId = bot.user?.id ?? "";
  const [handle, setHandle] = useState("");
  const transfers = useBotTransfers();
  const open = transfers.data?.outgoing.find((transfer) => transfer.bot?.id === botId);
  const withdraw = useCancel("bots.transfer.withdrawn");
  const send = useMutation({
    mutationFn: async () => {
      const value = handle.trim().replace(/^@/, "");
      // A handle nobody has reads as someone this bot cannot go to, like the server's answer.
      const person = await profileClient()
        .getProfile({ by: { case: "handle", value } })
        .catch(() => undefined);
      const userId = person?.profile?.user?.id ?? "";
      return botClient().offerBotTransfer({ botId, userId });
    },
    onSuccess: async () => {
      setHandle("");
      toast.success(t("bots.transfer.sent"));
      await refresh();
    },
    onError,
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    send.mutate();
  };

  return (
    <Section title={t("bots.transfer.title")}>
      <Card className="flex flex-col gap-3 p-4">
        <p className="text-sm leading-relaxed text-ink-soft">{t("bots.transfer.lead")}</p>
        <Notice>{t("bots.transfer.inheritsOffer")}</Notice>
        {open ? (
          <div className="flex flex-wrap items-center gap-3" data-testid="bot-transfer-pending">
            <p className="min-w-0 flex-1 basis-48 text-sm font-bold">
              {t("bots.transfer.pending", { to: open.to?.displayName ?? "" })}
            </p>
            <Button
              size="sm"
              variant="secondary"
              loading={withdraw.isPending}
              onClick={() => withdraw.mutate(open.id)}
            >
              {t("bots.transfer.withdraw")}
            </Button>
          </div>
        ) : (
          <form onSubmit={submit} className="flex flex-col gap-3">
            <TextField
              label={t("bots.transfer.to")}
              hint={t("bots.transfer.toHint")}
              value={handle}
              onChange={(event) => setHandle(event.target.value)}
              maxLength={25}
              autoCapitalize="none"
              autoComplete="off"
              spellCheck={false}
              required
            />
            <Button
              type="submit"
              variant="secondary"
              icon={<ArrowRightLeft className="size-4" />}
              loading={send.isPending}
              disabled={!handle.trim() || bot.suspended}
            >
              {t("bots.transfer.send")}
            </Button>
          </form>
        )}
      </Card>
    </Section>
  );
}
