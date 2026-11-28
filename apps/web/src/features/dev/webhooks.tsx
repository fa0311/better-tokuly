import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronRight, Plus, RefreshCw, RotateCw, Send, Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import {
  dev,
  eventTypesQuery,
  keys,
  localized,
  nextCursorOf,
  PAGE_SIZE,
  webhookClient,
} from "./api";
import {
  Ago,
  Fact,
  Notice,
  type RevealedSecret,
  SecretDialog,
  Section,
  useSecretAction,
  When,
} from "./parts";
import { CodeBlock } from "./reference/code-block";

/** The event sent by "send a test": nobody subscribes to it. */
const PING = "ping";

interface WebhookDraft {
  url: string;
  description: string;
  eventTypes: string[];
}

const draftReady = (draft: WebhookDraft) => draft.url.trim() !== "" && draft.eventTypes.length > 0;

/** URL, description and the events to receive. */
function WebhookFields({
  draft,
  onChange,
}: {
  draft: WebhookDraft;
  onChange(draft: WebhookDraft): void;
}) {
  const { t } = useTranslation("dev");
  const catalogue = useQuery(eventTypesQuery);
  const toggle = (type: string, on: boolean) =>
    onChange({
      ...draft,
      eventTypes: on
        ? [...draft.eventTypes, type]
        : draft.eventTypes.filter((other) => other !== type),
    });
  return (
    <>
      <TextField
        label={t("webhooks.url")}
        hint={t("webhooks.urlHint")}
        value={draft.url}
        onChange={(event) => onChange({ ...draft, url: event.target.value })}
        placeholder="https://example.com/tokuly/webhook"
        inputMode="url"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={2000}
        required
      />
      <TextField
        label={t("webhooks.description")}
        value={draft.description}
        onChange={(event) => onChange({ ...draft, description: event.target.value })}
        maxLength={200}
      />
      <fieldset>
        <legend className="mb-1.5 px-1 text-sm font-bold text-ink-soft">
          {t("webhooks.events")}
        </legend>
        {catalogue.isPending ? (
          <Skeleton className="h-48" />
        ) : catalogue.isError ? (
          <Notice tone="danger">{errorMessage(catalogue.error, { namespace: "dev" })}</Notice>
        ) : (
          <ul className="divide-y-2 divide-line rounded-2xl border-2 border-line">
            {catalogue.data
              .filter((event) => event.type !== PING)
              .map((event) => (
                <li key={event.type}>
                  <label className="flex cursor-pointer items-start gap-3 px-3 py-2.5 hover:bg-surface-2">
                    <input
                      type="checkbox"
                      className="mt-1 size-4 accent-brand"
                      checked={draft.eventTypes.includes(event.type)}
                      onChange={(change) => toggle(event.type, change.target.checked)}
                    />
                    <span className="min-w-0">
                      <span className="block font-mono text-sm font-bold">{event.type}</span>
                      <span className="block text-xs leading-relaxed text-ink-soft">
                        {localized(event.description)}
                      </span>
                    </span>
                  </label>
                </li>
              ))}
          </ul>
        )}
      </fieldset>
    </>
  );
}

function StatusChip({ status }: { status: dev.WebhookStatus }) {
  const { t } = useTranslation("dev");
  return (
    <Chip
      className={cn(
        status === dev.WebhookStatus.ACTIVE && "bg-lime text-[#17122b]",
        status === dev.WebhookStatus.DISABLED && "bg-danger/15 text-danger",
      )}
    >
      {t(`webhooks.status.${status}`)}
    </Chip>
  );
}

function DeliveryChip({ status }: { status: dev.DeliveryStatus }) {
  const { t } = useTranslation("dev");
  return (
    <Chip
      className={cn(
        status === dev.DeliveryStatus.SUCCEEDED && "bg-lime text-[#17122b]",
        status === dev.DeliveryStatus.RETRYING && "bg-sun text-[#17122b]",
        status === dev.DeliveryStatus.FAILED && "bg-danger/15 text-danger",
      )}
    >
      {t(`webhooks.delivery.status.${status}`)}
    </Chip>
  );
}

/** `/developers/webhooks`: the caller's webhooks, or (`?bot=`) those of one of their bots. */
export function WebhooksScreen({ botId }: { botId: string }) {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<WebhookDraft | null>(null);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);
  const [createdId, setCreatedId] = useState<string>();

  const webhooks = useQuery({
    queryKey: keys.webhooks(botId),
    queryFn: async () => (await webhookClient().listWebhooks({ botId })).webhooks,
  });
  const open = (webhookId: string) =>
    navigate({ to: "/developers/webhooks/$webhookId", params: { webhookId } });

  const create = useSecretAction(
    (form: WebhookDraft) => webhookClient().createWebhook({ ...form, url: form.url.trim(), botId }),
    (response) => {
      setDraft(null);
      setCreatedId(response.webhook?.id);
      setRevealed({
        title: t("webhooks.secretTitle"),
        lead: t("webhooks.secretLead"),
        secret: response.secret,
      });
      void queryClient.invalidateQueries({ queryKey: keys.webhooks(botId) });
    },
  );
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (draft && draftReady(draft)) void create.run(draft);
  };
  const start = () => setDraft({ url: "", description: "", eventTypes: [] });

  return (
    <Page
      title={t("webhooks.title")}
      back={botId ? "/developers/bots" : "/developers"}
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={start}>
          {t("webhooks.new")}
        </Button>
      }
    >
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">
        {botId ? t("webhooks.leadBot") : t("webhooks.lead")}
      </p>
      {webhooks.isPending ? (
        <Skeleton className="h-40" />
      ) : webhooks.isError ? (
        <Notice tone="danger">{errorMessage(webhooks.error, { namespace: "dev" })}</Notice>
      ) : webhooks.data.length === 0 ? (
        <EmptyState
          emoji="🪝"
          title={t("webhooks.empty")}
          body={t("webhooks.emptyBody")}
          action={<Button onClick={start}>{t("webhooks.new")}</Button>}
        />
      ) : (
        <Card className="p-2">
          <ul>
            {webhooks.data.map((webhook) => (
              <li key={webhook.id}>
                <ListRow
                  onClick={() => open(webhook.id)}
                  title={<span className="font-mono text-sm">{webhook.url}</span>}
                  subtitle={
                    webhook.description ||
                    t("webhooks.eventCount", { count: webhook.eventTypes.length })
                  }
                  trailing={
                    <>
                      <StatusChip status={webhook.status} />
                      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
                    </>
                  }
                />
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Dialog
        open={draft !== null}
        onOpenChange={(next) => !next && setDraft(null)}
        title={t("webhooks.newTitle")}
        footer={
          <Button
            type="submit"
            form="dev-webhook-form"
            loading={create.busy}
            disabled={!draft || !draftReady(draft)}
          >
            {t("webhooks.create")}
          </Button>
        }
      >
        {draft && (
          <form id="dev-webhook-form" onSubmit={submit} className="flex flex-col gap-4">
            <WebhookFields draft={draft} onChange={setDraft} />
          </form>
        )}
      </Dialog>
      <SecretDialog
        revealed={revealed}
        onClose={() => {
          setRevealed(null);
          if (createdId) void open(createdId);
        }}
      />
    </Page>
  );
}

/** `/developers/webhooks/<id>`: one webhook: its state, settings and delivery log. */
export function WebhookScreen({ webhookId }: { webhookId: string }) {
  const { t } = useTranslation("dev");
  const webhook = useQuery({
    queryKey: keys.webhook(webhookId),
    queryFn: async () => (await webhookClient().getWebhook({ webhookId })).webhook,
  });
  return (
    <Page title={t("webhooks.one")} back="/developers/webhooks">
      {webhook.isPending ? (
        <Skeleton className="h-96" />
      ) : webhook.isError || !webhook.data ? (
        <EmptyState emoji="🫥" title={errorMessage(webhook.error, { namespace: "dev" })} />
      ) : (
        <WebhookDetail webhook={webhook.data} />
      )}
    </Page>
  );
}

function WebhookDetail({ webhook }: { webhook: dev.Webhook }) {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<WebhookDraft>({
    url: webhook.url,
    description: webhook.description,
    eventTypes: webhook.eventTypes,
  });
  const [rotating, setRotating] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);
  const [shownDelivery, setShownDelivery] = useState<string>();
  const active = webhook.status === dev.WebhookStatus.ACTIVE;

  const store = (next: dev.Webhook | undefined) => {
    if (next) queryClient.setQueryData(keys.webhook(webhook.id), next);
    void queryClient.invalidateQueries({ queryKey: ["dev", "webhooks"] });
  };
  const refreshLog = () =>
    queryClient.invalidateQueries({ queryKey: [...keys.webhook(webhook.id), "deliveries"] });
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "dev" }));

  // The update replaces every field: what is not being changed is sent as it is saved.
  const update = useMutation({
    mutationFn: (change: Partial<WebhookDraft> & { paused: boolean }) =>
      webhookClient().updateWebhook({
        webhookId: webhook.id,
        url: webhook.url,
        description: webhook.description,
        eventTypes: webhook.eventTypes,
        ...change,
      }),
    onSuccess: (response) => store(response.webhook),
    onError,
  });

  const ping = useMutation({
    mutationFn: () => webhookClient().pingWebhook({ webhookId: webhook.id }),
    onSuccess: (response) => {
      const delivery = response.delivery;
      if (delivery) {
        queryClient.setQueryData(keys.delivery(delivery.id), delivery);
        setShownDelivery(delivery.id);
      }
      void refreshLog();
      void queryClient.invalidateQueries({ queryKey: keys.webhook(webhook.id) });
    },
    onError,
  });

  const rotate = useSecretAction(
    () => webhookClient().rotateWebhookSecret({ webhookId: webhook.id }),
    (response) => {
      store(response.webhook);
      setRotating(false);
      setRevealed({
        title: t("webhooks.secretTitle"),
        lead: t("webhooks.secretLead"),
        secret: response.secret,
      });
    },
  );

  const remove = useMutation({
    mutationFn: () => webhookClient().deleteWebhook({ webhookId: webhook.id }),
    onSuccess: async () => {
      toast.success(t("webhooks.deleted"));
      queryClient.removeQueries({ queryKey: keys.webhook(webhook.id) });
      await queryClient.invalidateQueries({ queryKey: ["dev", "webhooks"] });
      await navigate({ to: "/developers/webhooks", replace: true });
    },
    onError,
  });

  return (
    <>
      <Card className="p-4">
        <p className="flex flex-wrap items-center gap-2">
          <StatusChip status={webhook.status} />
          {webhook.consecutiveFailures > 0 && (
            <span className="text-xs font-bold text-danger">
              {t("webhooks.failures", { count: webhook.consecutiveFailures })}
            </span>
          )}
        </p>
        <p className="mt-2 font-mono text-sm font-bold break-all">{webhook.url}</p>
        {webhook.description && <p className="mt-1 text-sm text-ink-soft">{webhook.description}</p>}
        <dl className="mt-3">
          <Fact label={t("webhooks.secret")}>
            <code className="font-mono text-xs">{webhook.secretHint}</code>
            {webhook.previousSecretExpiresAt && (
              <span className="mt-0.5 block text-xs text-ink-soft">
                {t("webhooks.previousSecretUntil")} <When at={webhook.previousSecretExpiresAt} />
              </span>
            )}
          </Fact>
          <Fact label={t("webhooks.lastSuccess")}>
            {webhook.lastSuccessAt ? <Ago at={webhook.lastSuccessAt} /> : t("webhooks.never")}
          </Fact>
          <Fact label={t("webhooks.lastFailure")}>
            {webhook.lastFailureAt ? <Ago at={webhook.lastFailureAt} /> : t("webhooks.never")}
          </Fact>
        </dl>
        <div className="mt-3 flex flex-wrap gap-2">
          <Button
            size="sm"
            icon={<Send className="size-4" />}
            loading={ping.isPending}
            onClick={() => ping.mutate()}
          >
            {t("webhooks.ping")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            icon={<RefreshCw className="size-4" />}
            onClick={() => setRotating(true)}
          >
            {t("webhooks.rotate")}
          </Button>
          <Link
            to="/docs/api"
            hash="webhooks"
            className="inline-flex h-9 items-center rounded-full px-3 text-sm font-bold text-brand underline-offset-2 hover:underline"
          >
            {t("webhooks.howToVerify")}
          </Link>
        </div>
      </Card>

      {webhook.status === dev.WebhookStatus.DISABLED ? (
        <Notice tone="danger" className="mt-3">
          <span className="block">
            {t("webhooks.disabledBody", {
              reason: webhook.disabledReason || t("webhooks.disabledDefault"),
            })}
          </span>
          <Button
            size="sm"
            variant="secondary"
            className="mt-2"
            loading={update.isPending}
            onClick={() => update.mutate({ paused: false })}
          >
            {t("webhooks.resume")}
          </Button>
        </Notice>
      ) : (
        <Card className="mt-3 p-1.5">
          <SwitchRow
            label={t("webhooks.receiving")}
            hint={t("webhooks.receivingHint")}
            checked={active}
            disabled={update.isPending}
            onChange={(on) => update.mutate({ paused: !on })}
          />
        </Card>
      )}

      <Section title={t("webhooks.log")}>
        <DeliveryLog webhookId={webhook.id} onOpen={setShownDelivery} />
      </Section>

      <Section title={t("webhooks.settings")}>
        <Card className="p-4">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (!draftReady(draft)) return;
              update.mutate(
                { ...draft, url: draft.url.trim(), paused: !active },
                { onSuccess: () => toast.success(t("webhooks.saved")) },
              );
            }}
            className="flex flex-col gap-4"
          >
            <WebhookFields draft={draft} onChange={setDraft} />
            <Button type="submit" loading={update.isPending} disabled={!draftReady(draft)}>
              {t("webhooks.save")}
            </Button>
          </form>
        </Card>
      </Section>

      <Section title={t("webhooks.danger")}>
        <Card className="flex flex-wrap items-center gap-3 p-4">
          <p className="min-w-0 flex-1 basis-56 text-sm leading-relaxed text-ink-soft">
            {t("webhooks.deleteLead")}
          </p>
          <Button
            variant="danger"
            size="sm"
            icon={<Trash2 className="size-4" />}
            onClick={() => setDeleting(true)}
          >
            {t("webhooks.delete")}
          </Button>
        </Card>
      </Section>

      <ConfirmDialog
        open={rotating}
        onOpenChange={setRotating}
        title={t("webhooks.rotateTitle")}
        description={t("webhooks.rotateBody")}
        confirmLabel={t("webhooks.rotate")}
        loading={rotate.busy}
        onConfirm={() => void rotate.run(undefined)}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("webhooks.deleteTitle")}
        description={t("webhooks.deleteBody")}
        confirmLabel={t("webhooks.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <SecretDialog revealed={revealed} onClose={() => setRevealed(null)} />
      <DeliveryDialog
        deliveryId={shownDelivery}
        onClose={() => setShownDelivery(undefined)}
        onReplayed={(delivery) => {
          queryClient.setQueryData(keys.delivery(delivery.id), delivery);
          setShownDelivery(delivery.id);
          void refreshLog();
        }}
      />
    </>
  );
}

const LOG_FILTERS = [
  dev.DeliveryStatus.UNSPECIFIED,
  dev.DeliveryStatus.SUCCEEDED,
  dev.DeliveryStatus.RETRYING,
  dev.DeliveryStatus.FAILED,
] as const;

/** What was sent to a webhook, newest first. */
function DeliveryLog({ webhookId, onOpen }: { webhookId: string; onOpen(id: string): void }) {
  const { t } = useTranslation("dev");
  const [status, setStatus] = useState<dev.DeliveryStatus>(dev.DeliveryStatus.UNSPECIFIED);
  const deliveries = useInfiniteQuery({
    queryKey: keys.deliveries(webhookId, status),
    queryFn: ({ pageParam }) =>
      webhookClient().listDeliveries({
        webhookId,
        status,
        page: { cursor: pageParam, limit: PAGE_SIZE },
      }),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  const rows = deliveries.data?.pages.flatMap((page) => page.deliveries) ?? [];
  return (
    <>
      <div className="mb-2 flex items-center gap-2">
        <Tabs
          items={LOG_FILTERS.map((value) => ({
            value: String(value),
            label: t(`webhooks.delivery.filter.${value}`),
          }))}
          value={String(status)}
          onChange={(next) => setStatus(Number(next) as dev.DeliveryStatus)}
          className="min-w-0"
        />
        <span className="flex-1" />
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t("webhooks.refresh")}
          loading={deliveries.isRefetching}
          icon={<RotateCw className="size-4" />}
          onClick={() => void deliveries.refetch()}
        />
      </div>
      {deliveries.isPending ? (
        <Skeleton className="h-32" />
      ) : deliveries.isError ? (
        <Notice tone="danger">{errorMessage(deliveries.error, { namespace: "dev" })}</Notice>
      ) : rows.length === 0 ? (
        <Notice>{t("webhooks.logEmpty")}</Notice>
      ) : (
        <Card className="p-2">
          <ul aria-label={t("webhooks.log")}>
            {rows.map((delivery) => {
              const last = delivery.attempts.at(-1);
              return (
                <li key={delivery.id}>
                  <ListRow
                    onClick={() => onOpen(delivery.id)}
                    title={<span className="font-mono text-sm">{delivery.eventType}</span>}
                    subtitle={
                      <>
                        <Ago at={delivery.createdAt} />
                        {last &&
                          ` · ${last.statusCode > 0 ? `HTTP ${last.statusCode}` : last.error}`}
                        {delivery.replayOf && ` · ${t("webhooks.delivery.replay")}`}
                      </>
                    }
                    trailing={<DeliveryChip status={delivery.status} />}
                  />
                </li>
              );
            })}
          </ul>
          <LoadMore query={deliveries} />
        </Card>
      )}
    </>
  );
}

/** One delivery: what was sent, every attempt and what the endpoint answered; and sending it again. */
function DeliveryDialog({
  deliveryId,
  onClose,
  onReplayed,
}: {
  deliveryId: string | undefined;
  onClose(): void;
  onReplayed(delivery: dev.Delivery): void;
}) {
  const { t } = useTranslation("dev");
  const delivery = useQuery({
    queryKey: keys.delivery(deliveryId ?? ""),
    queryFn: async () =>
      (await webhookClient().getDelivery({ deliveryId: deliveryId ?? "" })).delivery,
    enabled: deliveryId !== undefined,
  });
  const replay = useMutation({
    mutationFn: () => webhookClient().replayDelivery({ deliveryId: deliveryId ?? "" }),
    onSuccess: (response) => {
      toast.success(t("webhooks.delivery.replayed"));
      if (response.delivery) onReplayed(response.delivery);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });
  const data = delivery.data;
  return (
    <Dialog
      open={deliveryId !== undefined}
      onOpenChange={(open) => !open && onClose()}
      title={t("webhooks.delivery.title")}
      footer={
        <Button
          variant="secondary"
          icon={<RotateCw className="size-4" />}
          loading={replay.isPending}
          disabled={!data}
          onClick={() => replay.mutate()}
        >
          {t("webhooks.delivery.replayAction")}
        </Button>
      }
    >
      {delivery.isPending ? (
        <Skeleton className="h-64" />
      ) : delivery.isError || !data ? (
        <Notice tone="danger">{errorMessage(delivery.error, { namespace: "dev" })}</Notice>
      ) : (
        <div className="flex flex-col gap-4">
          <div>
            <p className="flex flex-wrap items-center gap-2">
              <code className="font-mono text-sm font-bold">{data.eventType}</code>
              <DeliveryChip status={data.status} />
              {data.replayOf && <Chip>{t("webhooks.delivery.replay")}</Chip>}
            </p>
            <dl className="mt-2">
              <Fact label={t("webhooks.delivery.id")}>
                <code className="font-mono text-xs break-all">{data.id}</code>
              </Fact>
              <Fact label={t("webhooks.delivery.eventId")}>
                <code className="font-mono text-xs break-all">{data.eventId}</code>
              </Fact>
              <Fact label={t("webhooks.delivery.created")}>
                <When at={data.createdAt} />
              </Fact>
              {data.nextAttemptAt && (
                <Fact label={t("webhooks.delivery.nextAttempt")}>
                  <When at={data.nextAttemptAt} />
                </Fact>
              )}
            </dl>
          </div>

          {data.payload && (
            <CodeBlock
              language="json"
              label={t("webhooks.delivery.payload")}
              code={prettyJson(data.payload)}
              maxHeight="max-h-64"
            />
          )}

          <div>
            <p className="mb-1.5 text-sm font-black">
              {t("webhooks.delivery.attempts", { count: data.attemptCount })}
            </p>
            {data.attempts.length === 0 ? (
              <Notice>{t("webhooks.delivery.noAttempts")}</Notice>
            ) : (
              <ol className="flex flex-col gap-2">
                {data.attempts.map((attempt, index) => (
                  <li key={index} className="rounded-2xl bg-surface-2 px-3 py-2.5 text-sm">
                    <p className="flex flex-wrap items-center gap-x-2 gap-y-0.5 font-bold">
                      <span
                        className={cn(
                          "font-mono",
                          attempt.statusCode >= 200 && attempt.statusCode < 300
                            ? "text-success"
                            : "text-danger",
                        )}
                      >
                        {attempt.statusCode > 0 ? `HTTP ${attempt.statusCode}` : attempt.error}
                      </span>
                      <span className="text-xs text-ink-soft tabular-nums">
                        {attempt.latencyMs} ms
                      </span>
                      <span className="text-xs text-ink-soft">
                        <When at={attempt.at} />
                      </span>
                    </p>
                    {attempt.responseSnippet && (
                      <pre className="mt-1.5 max-h-32 overflow-auto font-mono text-xs break-all whitespace-pre-wrap text-ink-soft">
                        {attempt.responseSnippet}
                      </pre>
                    )}
                  </li>
                ))}
              </ol>
            )}
          </div>
        </div>
      )}
    </Dialog>
  );
}

/** The payload as sent, indented for reading; as it came when it is not JSON. */
function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}
