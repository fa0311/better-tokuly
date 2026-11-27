import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { media } from "@tokuly/api";
import { ChevronRight, Plus, Trash2, Webhook } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { botClient, type dev, keys } from "./api";
import { BotTransferInbox, HandOverSection } from "./bot-transfers";
import { ImageField } from "./image-field";
import { Notice, type RevealedSecret, SecretDialog, Section, useSecretAction } from "./parts";
import { allowedForBots } from "./scopes";
import { TokenCard, type TokenDraft, TokenFormDialog } from "./tokens";
import { UsagePanel } from "./usage";

function BotChips({ bot }: { bot: dev.Bot }) {
  const { t } = useTranslation("dev");
  return (
    <>
      <Chip className="bg-sky text-[#17122b]">BOT</Chip>
      {bot.suspended ? (
        <Chip className="bg-danger/15 text-danger">{t("bots.suspended")}</Chip>
      ) : (
        bot.disabled && <Chip>{t("bots.off")}</Chip>
      )}
    </>
  );
}

/** `/developers/bots`: the caller's bots. */
export function BotsScreen() {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const [handle, setHandle] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [bio, setBio] = useState("");

  const bots = useQuery({ queryKey: keys.bots, queryFn: () => botClient().listBots({}) });

  const create = useMutation({
    mutationFn: () => botClient().createBot({ handle: handle.trim(), displayName, bio }),
    onSuccess: async (response) => {
      setCreating(false);
      setHandle("");
      setDisplayName("");
      setBio("");
      await queryClient.invalidateQueries({ queryKey: keys.bots });
      const botId = response.bot?.user?.id;
      if (botId) await navigate({ to: "/developers/bots/$botId", params: { botId } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    create.mutate();
  };

  const rows = bots.data?.bots ?? [];
  const full = bots.data !== undefined && rows.length >= bots.data.maxBots;
  const newButton = (
    <Button
      size="sm"
      icon={<Plus className="size-4" />}
      disabled={full}
      onClick={() => setCreating(true)}
    >
      {t("bots.new")}
    </Button>
  );
  return (
    <Page title={t("bots.title")} back="/developers" actions={newButton}>
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("bots.lead")}</p>
      <BotTransferInbox />
      {bots.isPending ? (
        <Skeleton className="h-40" />
      ) : bots.isError ? (
        <Notice tone="danger">{errorMessage(bots.error, { namespace: "dev" })}</Notice>
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🤖"
          title={t("bots.empty")}
          body={t("bots.emptyBody")}
          action={<Button onClick={() => setCreating(true)}>{t("bots.new")}</Button>}
        />
      ) : (
        <>
          <Card className="p-2">
            <ul>
              {rows.map((bot) => (
                <li key={bot.user?.id}>
                  <ListRow
                    onClick={() =>
                      navigate({
                        to: "/developers/bots/$botId",
                        params: { botId: bot.user?.id ?? "" },
                      })
                    }
                    leading={<Avatar user={bot.user} />}
                    title={
                      <span className="flex items-center gap-2">
                        <span className="truncate">
                          <EmojiText text={bot.user?.displayName ?? ""} />
                        </span>
                        <BotChips bot={bot} />
                      </span>
                    }
                    subtitle={`@${bot.user?.handle} · ${t("bots.tokenCount", { count: bot.tokenCount })}`}
                    trailing={<ChevronRight className="size-5 shrink-0 text-ink-faint" />}
                  />
                </li>
              ))}
            </ul>
          </Card>
          <p className="mt-2 px-1 text-xs font-bold text-ink-faint">
            {t("bots.allowance", { count: rows.length, max: bots.data.maxBots })}
          </p>
        </>
      )}

      <Dialog
        open={creating}
        onOpenChange={setCreating}
        title={t("bots.newTitle")}
        description={t("bots.newLead")}
        footer={
          <Button
            type="submit"
            form="dev-bot-form"
            loading={create.isPending}
            disabled={!handle.trim() || !displayName.trim()}
          >
            {t("bots.create")}
          </Button>
        }
      >
        <form id="dev-bot-form" onSubmit={submit} className="flex flex-col gap-4">
          <TextField
            label={t("bots.handle")}
            hint={t("bots.handleHint")}
            value={handle}
            onChange={(event) => setHandle(event.target.value.toLowerCase())}
            maxLength={24}
            autoCapitalize="none"
            autoComplete="off"
            spellCheck={false}
            required
          />
          <TextField
            label={t("bots.displayName")}
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            maxLength={40}
            autoComplete="off"
            required
          />
          <TextArea
            label={t("bots.bio")}
            value={bio}
            onChange={(event) => setBio(event.target.value)}
            maxLength={300}
          />
        </form>
      </Dialog>
    </Page>
  );
}

/** `/developers/bots/<id>`: one bot: its profile, switch, tokens, webhooks and usage. */
export function BotScreen({ botId }: { botId: string }) {
  const { t } = useTranslation("dev");
  const bot = useQuery({
    queryKey: keys.bot(botId),
    queryFn: async () => (await botClient().getBot({ botId })).bot,
  });
  return (
    <Page title={bot.data?.user?.displayName ?? t("bots.title")} back="/developers/bots">
      {bot.isPending ? (
        <Skeleton className="h-96" />
      ) : bot.isError || !bot.data ? (
        <EmptyState emoji="🫥" title={errorMessage(bot.error, { namespace: "dev" })} />
      ) : (
        <BotDetail bot={bot.data} />
      )}
    </Page>
  );
}

function BotDetail({ bot }: { bot: dev.Bot }) {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const botId = bot.user?.id ?? "";
  const [displayName, setDisplayName] = useState(bot.user?.displayName ?? "");
  const [handle, setHandle] = useState(bot.user?.handle ?? "");
  const [bio, setBio] = useState(bot.bio);
  const [avatar, setAvatar] = useState<string>();
  const [deleting, setDeleting] = useState(false);

  const store = (next: dev.Bot | undefined) => {
    if (next) queryClient.setQueryData(keys.bot(botId), next);
    void queryClient.invalidateQueries({ queryKey: keys.bots });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "dev" }));

  const save = useMutation({
    mutationFn: () =>
      botClient().updateBot({
        botId,
        displayName,
        bio,
        // A handle can only be changed a few times a week: send it when it changed.
        handle: handle.trim() === bot.user?.handle ? undefined : handle.trim(),
        avatarAssetId: avatar,
      }),
    onSuccess: (response) => {
      store(response.bot);
      setAvatar(undefined);
      toast.success(t("bots.saved"));
    },
    onError,
  });

  const toggle = useMutation({
    mutationFn: (disabled: boolean) => botClient().setBotDisabled({ botId, disabled }),
    onSuccess: (response) => store(response.bot),
    onError,
  });

  const remove = useMutation({
    mutationFn: () => botClient().deleteBot({ botId }),
    onSuccess: async () => {
      toast.success(t("bots.deleted"));
      queryClient.removeQueries({ queryKey: keys.bot(botId) });
      await queryClient.invalidateQueries({ queryKey: keys.bots });
      await navigate({ to: "/developers/bots", replace: true });
    },
    onError,
  });

  return (
    <>
      <Card className="flex items-center gap-4">
        <Avatar user={bot.user} size="lg" outlined />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-xl font-black">
            <span className="min-w-0 break-words">
              <EmojiText text={bot.user?.displayName ?? ""} />
            </span>
            <BotChips bot={bot} />
          </p>
          <Link
            to="/u/$handle"
            params={{ handle: bot.user?.handle ?? "" }}
            className="text-sm font-bold text-ink-soft underline-offset-2 hover:underline"
          >
            @{bot.user?.handle}
          </Link>
        </div>
      </Card>
      {bot.suspended && (
        <Notice tone="danger" className="mt-3">
          {t("bots.suspendedBody")}
        </Notice>
      )}

      <Card className="mt-3 p-1.5">
        <SwitchRow
          label={t("bots.enabled")}
          hint={t("bots.enabledHint")}
          checked={!bot.disabled}
          disabled={toggle.isPending || bot.suspended}
          onChange={(enabled) => toggle.mutate(!enabled)}
        />
      </Card>

      <Section title={t("bots.tokens")}>
        <BotTokens botId={botId} />
      </Section>

      <Section title={t("webhooks.title")}>
        <Link
          to="/developers/webhooks"
          search={{ bot: botId }}
          className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-4"
        >
          <Webhook className="size-5 shrink-0" />
          <span className="min-w-0 flex-1 text-sm font-bold">{t("bots.webhooksLead")}</span>
          <ChevronRight className="size-5 shrink-0 text-ink-faint" />
        </Link>
      </Section>

      <Section title={t("bots.profile")}>
        <Card className="p-4">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              save.mutate();
            }}
            className="flex flex-col gap-4"
          >
            <ImageField
              label={t("bots.avatar")}
              name={displayName}
              current={bot.user?.avatar}
              purpose={media.AssetPurpose.AVATAR}
              value={avatar}
              onChange={setAvatar}
            />
            <TextField
              label={t("bots.displayName")}
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              maxLength={40}
              required
            />
            <TextField
              label={t("bots.handle")}
              hint={t("bots.handleChangeHint")}
              value={handle}
              onChange={(event) => setHandle(event.target.value.toLowerCase())}
              maxLength={24}
              autoCapitalize="none"
              spellCheck={false}
              required
            />
            <TextArea
              label={t("bots.bio")}
              value={bio}
              onChange={(event) => setBio(event.target.value)}
              maxLength={300}
            />
            <Button type="submit" loading={save.isPending}>
              {t("bots.save")}
            </Button>
          </form>
        </Card>
      </Section>

      <Section title={t("usage.title")}>
        <Card className="p-4">
          <UsagePanel subject={{ kind: "bot", id: botId }} />
        </Card>
      </Section>

      <HandOverSection bot={bot} />

      <Section title={t("bots.danger")}>
        <Card className="flex flex-wrap items-center gap-3 p-4">
          <p className="min-w-0 flex-1 basis-56 text-sm leading-relaxed text-ink-soft">
            {bot.suspended ? t("bots.deleteBlocked") : t("bots.deleteLead")}
          </p>
          <Button
            variant="danger"
            size="sm"
            icon={<Trash2 className="size-4" />}
            disabled={bot.suspended}
            onClick={() => setDeleting(true)}
          >
            {t("bots.delete")}
          </Button>
        </Card>
      </Section>

      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("bots.deleteTitle", { name: bot.user?.displayName })}
        description={t("bots.deleteBody")}
        confirmLabel={t("bots.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}

/** The tokens that act as a bot. */
function BotTokens({ botId }: { botId: string }) {
  const { t } = useTranslation("dev");
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);

  const tokens = useQuery({
    queryKey: keys.botTokens(botId),
    queryFn: async () => (await botClient().listBotTokens({ botId })).tokens,
  });
  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: keys.botTokens(botId) });
    void queryClient.invalidateQueries({ queryKey: keys.bot(botId) });
    void queryClient.invalidateQueries({ queryKey: keys.bots });
  };

  const create = useSecretAction(
    (draft: TokenDraft) => botClient().createBotToken({ botId, ...draft }),
    (response) => {
      setCreating(false);
      setRevealed({
        title: t("bots.tokenCreatedTitle"),
        lead: t("tokens.createdLead"),
        secret: response.secret,
      });
      refresh();
    },
  );
  const revoke = useMutation({
    mutationFn: (tokenId: string) => botClient().revokeBotToken({ tokenId }),
    onSuccess: () => {
      toast.success(t("tokens.revoked"));
      refresh();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });

  return (
    <>
      {tokens.isPending ? (
        <Skeleton className="h-24" />
      ) : tokens.isError ? (
        <Notice tone="danger">{errorMessage(tokens.error, { namespace: "dev" })}</Notice>
      ) : tokens.data.length === 0 ? (
        <Notice>{t("bots.noTokens")}</Notice>
      ) : (
        <ul className="flex flex-col gap-3">
          {tokens.data.map((token) => (
            <li key={token.id}>
              <TokenCard
                token={token}
                revoking={revoke.isPending && revoke.variables === token.id}
                onRevoke={() => revoke.mutate(token.id)}
              />
            </li>
          ))}
        </ul>
      )}
      <Button
        variant="secondary"
        size="sm"
        className="mt-3"
        icon={<Plus className="size-4" />}
        onClick={() => setCreating(true)}
      >
        {t("bots.newToken")}
      </Button>
      {creating && (
        <TokenFormDialog
          open
          onOpenChange={setCreating}
          title={t("bots.newTokenTitle")}
          lead={t("bots.newTokenLead")}
          busy={create.busy}
          onSubmit={create.run}
          offered={allowedForBots}
        />
      )}
      <SecretDialog revealed={revealed} onClose={() => setRevealed(null)} />
    </>
  );
}
