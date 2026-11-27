import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { media } from "@tokuly/api";
import { ChevronRight, Plus, RefreshCw, Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiText } from "#/features/emoji";
import { API_PATH } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { appClient, dev, keys, nextCursorOf, PAGE_SIZE } from "./api";
import { ImageField, SquareImage } from "./image-field";
import {
  Ago,
  CopyButton,
  Fact,
  Notice,
  type RevealedSecret,
  ScopeChips,
  SecretDialog,
  Section,
  useSecretAction,
  When,
} from "./parts";
import { CodeBlock } from "./reference/code-block";
import { ScopePicker } from "./scope-picker";
import { UsagePanel } from "./usage";

/** The editable fields of an application as the form holds them. */
interface AppDraft {
  name: string;
  description: string;
  homepageUrl: string;
  privacyPolicyUrl: string;
  /** One URI per line. */
  redirectUris: string;
  scopes: string[];
  clientType: dev.AppClientType;
  deviceFlow: boolean;
  /** `undefined` keeps the saved icon, `""` removes it. */
  iconAssetId: string | undefined;
}

const EMPTY_DRAFT: AppDraft = {
  name: "",
  description: "",
  homepageUrl: "",
  privacyPolicyUrl: "",
  redirectUris: "",
  scopes: [],
  clientType: dev.AppClientType.CONFIDENTIAL,
  deviceFlow: false,
  iconAssetId: undefined,
};

const draftOf = (app: dev.App): AppDraft => ({
  name: app.name,
  description: app.description,
  homepageUrl: app.homepageUrl,
  privacyPolicyUrl: app.privacyPolicyUrl,
  redirectUris: app.redirectUris.join("\n"),
  scopes: app.scopes,
  clientType: app.clientType,
  deviceFlow: app.deviceFlow,
  iconAssetId: undefined,
});

const linesOf = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

/** The fields of an application, for registering one and for editing it. */
function AppFields({
  draft,
  onChange,
  current,
}: {
  draft: AppDraft;
  onChange(draft: AppDraft): void;
  /** The saved application when editing: its client type is fixed. */
  current?: dev.App;
}) {
  const { t } = useTranslation("dev");
  const set = <K extends keyof AppDraft>(key: K, value: AppDraft[K]) =>
    onChange({ ...draft, [key]: value });
  return (
    <>
      <ImageField
        label={t("apps.icon")}
        name={draft.name}
        current={current?.icon}
        purpose={media.AssetPurpose.CHANNEL_ICON}
        value={draft.iconAssetId}
        onChange={(assetId) => set("iconAssetId", assetId)}
      />
      <TextField
        label={t("apps.name")}
        hint={t("apps.nameHint")}
        value={draft.name}
        onChange={(event) => set("name", event.target.value)}
        maxLength={60}
        autoComplete="off"
        required
      />
      <TextArea
        label={t("apps.description")}
        hint={t("apps.descriptionHint")}
        value={draft.description}
        onChange={(event) => set("description", event.target.value)}
        maxLength={500}
      />
      {!current && (
        <SelectField
          label={t("apps.clientType")}
          value={String(draft.clientType)}
          onChange={(next) => set("clientType", Number(next) as dev.AppClientType)}
          options={[dev.AppClientType.CONFIDENTIAL, dev.AppClientType.PUBLIC].map((type) => ({
            value: String(type),
            label: t(`apps.clientTypes.${type}.label`),
            hint: t(`apps.clientTypes.${type}.hint`),
          }))}
        />
      )}
      <TextArea
        label={t("apps.redirectUris")}
        hint={t("apps.redirectUrisHint")}
        value={draft.redirectUris}
        onChange={(event) => set("redirectUris", event.target.value)}
        placeholder="https://example.com/callback"
        spellCheck={false}
        autoCapitalize="none"
        required
      />
      <TextField
        label={t("apps.homepageUrl")}
        value={draft.homepageUrl}
        onChange={(event) => set("homepageUrl", event.target.value)}
        placeholder="https://"
        inputMode="url"
        autoCapitalize="none"
      />
      <TextField
        label={t("apps.privacyPolicyUrl")}
        value={draft.privacyPolicyUrl}
        onChange={(event) => set("privacyPolicyUrl", event.target.value)}
        placeholder="https://"
        inputMode="url"
        autoCapitalize="none"
      />
      <div className="rounded-2xl border-2 border-line">
        <SwitchRow
          label={t("apps.deviceFlow")}
          hint={t("apps.deviceFlowHint")}
          checked={draft.deviceFlow}
          onChange={(checked) => set("deviceFlow", checked)}
        />
      </div>
      <div>
        <p className="mb-1.5 px-1 text-sm font-bold text-ink-soft">{t("apps.scopes")}</p>
        <p className="mb-2 px-1 text-xs leading-relaxed text-ink-faint">{t("apps.scopesHint")}</p>
        <ScopePicker value={draft.scopes} onChange={(scopes) => set("scopes", scopes)} />
      </div>
    </>
  );
}

const draftReady = (draft: AppDraft) =>
  draft.name.trim().length > 0 && linesOf(draft.redirectUris).length > 0 && draft.scopes.length > 0;

function StatusChip({ app }: { app: dev.App }) {
  const { t } = useTranslation("dev");
  return app.status === dev.AppStatus.DISABLED ? (
    <Chip className="bg-danger/15 text-danger">{t("apps.disabled")}</Chip>
  ) : null;
}

/** `/developers/apps`: the OAuth applications the caller registered. */
export function AppsScreen() {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<AppDraft | null>(null);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);
  // Where to go once the secret of a new application has been put away.
  const [createdId, setCreatedId] = useState<string>();

  const apps = useInfiniteQuery({
    queryKey: keys.apps,
    queryFn: ({ pageParam }) =>
      appClient().listApps({ page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });

  const open = (appId: string) => navigate({ to: "/developers/apps/$appId", params: { appId } });
  const create = useSecretAction(
    (form: AppDraft) =>
      appClient().createApp({
        ...form,
        name: form.name.trim(),
        redirectUris: linesOf(form.redirectUris),
        iconAssetId: form.iconAssetId ?? "",
      }),
    (response) => {
      setDraft(null);
      void queryClient.invalidateQueries({ queryKey: keys.apps });
      const appId = response.app?.id ?? "";
      if (!response.clientSecret) return void open(appId);
      setCreatedId(appId);
      setRevealed({
        title: t("apps.secretTitle"),
        lead: t("apps.secretLead"),
        secret: response.clientSecret,
      });
    },
  );

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (draft && draftReady(draft)) void create.run(draft);
  };

  const rows = apps.data?.pages.flatMap((page) => page.apps) ?? [];
  return (
    <Page
      title={t("apps.title")}
      back="/developers"
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setDraft(EMPTY_DRAFT)}>
          {t("apps.new")}
        </Button>
      }
    >
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("apps.lead")}</p>
      {apps.isPending ? (
        <Skeleton className="h-48" />
      ) : apps.isError ? (
        <Notice tone="danger">{errorMessage(apps.error, { namespace: "dev" })}</Notice>
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🧩"
          title={t("apps.empty")}
          body={t("apps.emptyBody")}
          action={<Button onClick={() => setDraft(EMPTY_DRAFT)}>{t("apps.new")}</Button>}
        />
      ) : (
        <Card className="p-2">
          <ul>
            {rows.map((app) => (
              <li key={app.id}>
                <ListRow
                  onClick={() => open(app.id)}
                  leading={<SquareImage image={app.icon} name={app.name} />}
                  title={
                    <span className="flex items-center gap-2">
                      <span className="truncate">
                        <EmojiText text={app.name} />
                      </span>
                      <StatusChip app={app} />
                    </span>
                  }
                  subtitle={`${t(`apps.clientTypes.${app.clientType}.label`)} · ${t("apps.userCount", { count: app.userCount })}`}
                  trailing={<ChevronRight className="size-5 shrink-0 text-ink-faint" />}
                />
              </li>
            ))}
          </ul>
          <LoadMore query={apps} />
        </Card>
      )}

      <Dialog
        open={draft !== null}
        onOpenChange={(next) => !next && setDraft(null)}
        title={t("apps.newTitle")}
        footer={
          <Button
            type="submit"
            form="dev-app-form"
            loading={create.busy}
            disabled={!draft || !draftReady(draft)}
          >
            {t("apps.register")}
          </Button>
        }
      >
        {draft && (
          <form id="dev-app-form" onSubmit={submit} className="flex flex-col gap-4">
            <AppFields draft={draft} onChange={setDraft} />
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

/** `/developers/apps/<id>`: one application: its credentials, settings, users and usage. */
export function AppScreen({ appId }: { appId: string }) {
  const { t } = useTranslation("dev");
  const app = useQuery({
    queryKey: keys.app(appId),
    queryFn: async () => (await appClient().getApp({ appId })).app,
  });
  return (
    <Page title={app.data?.name ?? t("apps.title")} back="/developers/apps">
      {app.isPending ? (
        <Skeleton className="h-96" />
      ) : app.isError || !app.data ? (
        <EmptyState emoji="🫥" title={errorMessage(app.error, { namespace: "dev" })} />
      ) : (
        <AppDetail app={app.data} />
      )}
    </Page>
  );
}

function AppDetail({ app }: { app: dev.App }) {
  const { t } = useTranslation("dev");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(app));
  const [rotating, setRotating] = useState(false);
  const [revokePrevious, setRevokePrevious] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);
  const confidential = app.clientType === dev.AppClientType.CONFIDENTIAL;
  const disabled = app.status === dev.AppStatus.DISABLED;

  const store = (next: dev.App | undefined) => {
    if (next) queryClient.setQueryData(keys.app(app.id), next);
    void queryClient.invalidateQueries({ queryKey: keys.apps });
  };

  const save = useMutation({
    mutationFn: () => {
      // The client type is fixed when the application is registered.
      const { clientType: _fixed, ...editable } = draft;
      return appClient().updateApp({
        appId: app.id,
        ...editable,
        name: draft.name.trim(),
        redirectUris: linesOf(draft.redirectUris),
      });
    },
    onSuccess: (response) => {
      store(response.app);
      if (response.app) setDraft(draftOf(response.app));
      toast.success(t("apps.saved"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });

  const rotate = useSecretAction(
    () => appClient().rotateClientSecret({ appId: app.id, revokePrevious }),
    (response) => {
      store(response.app);
      setRotating(false);
      setRevealed({
        title: t("apps.secretTitle"),
        lead: t("apps.secretLead"),
        secret: response.clientSecret,
      });
    },
  );

  const remove = useMutation({
    mutationFn: () => appClient().deleteApp({ appId: app.id }),
    onSuccess: async () => {
      toast.success(t("apps.deleted"));
      queryClient.removeQueries({ queryKey: keys.app(app.id) });
      await queryClient.invalidateQueries({ queryKey: keys.apps });
      await navigate({ to: "/developers/apps", replace: true });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });

  const origin = window.location.origin;
  const authorizeUrl = `${origin}/oauth/authorize?response_type=code&client_id=${app.id}&redirect_uri=${encodeURIComponent(app.redirectUris[0] ?? "")}&scope=${encodeURIComponent(app.scopes.join(" "))}&state=<random>&code_challenge=<base64url(sha256(verifier))>&code_challenge_method=S256`;
  const tokenCall = [
    `curl ${origin}${API_PATH}/oauth/token \\`,
    ...(confidential ? ['  -u "$CLIENT_ID:$CLIENT_SECRET" \\'] : [`  -d client_id=${app.id} \\`]),
    "  -d grant_type=authorization_code \\",
    '  -d code="$CODE" \\',
    '  -d code_verifier="$VERIFIER" \\',
    `  -d redirect_uri=${app.redirectUris[0] ?? ""}`,
  ].join("\n");

  return (
    <>
      <Card className="flex items-center gap-4">
        <SquareImage image={app.icon} name={app.name} className="size-16" />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-xl font-black">
            <span className="min-w-0 break-words">
              <EmojiText text={app.name} />
            </span>
            <StatusChip app={app} />
          </p>
          <p className="text-sm font-bold text-ink-soft">
            {t(`apps.clientTypes.${app.clientType}.label`)} ·{" "}
            {t("apps.userCount", { count: app.userCount })}
          </p>
        </div>
      </Card>
      {disabled && (
        <Notice tone="danger" className="mt-3">
          {t("apps.disabledBody", { reason: app.disabledReason || t("apps.noReason") })}
        </Notice>
      )}

      <Section title={t("apps.credentials")}>
        <Card className="p-4">
          <dl>
            <Fact label={t("apps.clientId")}>
              <span className="flex flex-wrap items-center gap-2">
                <code className="font-mono text-xs break-all">{app.id}</code>
                <CopyButton text={app.id} />
              </span>
            </Fact>
            {confidential && (
              <Fact label={t("apps.clientSecret")}>
                <code className="font-mono text-xs">{app.clientSecretHint}</code>
                {app.previousSecretExpiresAt && (
                  <span className="mt-0.5 block text-xs text-ink-soft">
                    {t("apps.previousSecretUntil")} <When at={app.previousSecretExpiresAt} />
                  </span>
                )}
              </Fact>
            )}
            <Fact label={t("apps.created")}>
              <When at={app.createdAt} />
            </Fact>
          </dl>
          {confidential && (
            <div className="mt-2 flex justify-end">
              <Button
                variant="secondary"
                size="sm"
                icon={<RefreshCw className="size-4" />}
                onClick={() => {
                  setRevokePrevious(false);
                  setRotating(true);
                }}
              >
                {t("apps.rotate")}
              </Button>
            </div>
          )}
        </Card>
      </Section>

      <Section title={t("apps.connect")}>
        <Card className="flex flex-col gap-3 p-4">
          <p className="text-sm leading-relaxed text-ink-soft">{t("apps.connectLead")}</p>
          <CodeBlock language="text" code={authorizeUrl} label={t("apps.authorizeUrl")} />
          <CodeBlock language="shell" code={tokenCall} label={t("apps.tokenCall")} />
          <p className="text-sm leading-relaxed text-ink-soft">
            {t("apps.metadata")}{" "}
            <code className="font-mono text-xs break-all">
              {origin}
              {API_PATH}/.well-known/oauth-authorization-server
            </code>
          </p>
        </Card>
      </Section>

      <Section title={t("apps.settings")}>
        <Card className="p-4">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (draftReady(draft)) save.mutate();
            }}
            className="flex flex-col gap-4"
          >
            <AppFields draft={draft} onChange={setDraft} current={app} />
            <Button type="submit" loading={save.isPending} disabled={!draftReady(draft)}>
              {t("apps.save")}
            </Button>
          </form>
        </Card>
      </Section>

      <Section title={t("apps.users")}>
        <AppUsers appId={app.id} />
      </Section>

      <Section title={t("usage.title")}>
        <Card className="p-4">
          <UsagePanel subject={{ kind: "app", id: app.id }} />
        </Card>
      </Section>

      <Section title={t("apps.danger")}>
        <Card className="flex flex-wrap items-center gap-3 p-4">
          <p className="min-w-0 flex-1 basis-56 text-sm leading-relaxed text-ink-soft">
            {disabled ? t("apps.deleteBlocked") : t("apps.deleteLead")}
          </p>
          <Button
            variant="danger"
            size="sm"
            icon={<Trash2 className="size-4" />}
            disabled={disabled}
            onClick={() => setDeleting(true)}
          >
            {t("apps.delete")}
          </Button>
        </Card>
      </Section>

      <ConfirmDialog
        open={rotating}
        onOpenChange={setRotating}
        title={t("apps.rotateTitle")}
        description={t("apps.rotateBody")}
        confirmLabel={t("apps.rotate")}
        loading={rotate.busy}
        onConfirm={() => void rotate.run(undefined)}
      >
        <SwitchRow
          label={t("apps.revokePrevious")}
          hint={t("apps.revokePreviousHint")}
          checked={revokePrevious}
          onChange={setRevokePrevious}
        />
      </ConfirmDialog>
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("apps.deleteTitle", { name: app.name })}
        description={t("apps.deleteBody")}
        confirmLabel={t("apps.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <SecretDialog revealed={revealed} onClose={() => setRevealed(null)} />
    </>
  );
}

/** Who authorized the application, most recently active first. */
function AppUsers({ appId }: { appId: string }) {
  const { t } = useTranslation("dev");
  const users = useInfiniteQuery({
    queryKey: keys.appUsers(appId),
    queryFn: ({ pageParam }) =>
      appClient().listAppUsers({ appId, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });
  if (users.isPending) return <Skeleton className="h-24" />;
  if (users.isError)
    return <Notice tone="danger">{errorMessage(users.error, { namespace: "dev" })}</Notice>;
  const rows = users.data.pages.flatMap((page) => page.users);
  if (rows.length === 0) return <Notice>{t("apps.noUsers")}</Notice>;
  return (
    <Card className="p-2">
      <ul>
        {rows.map((row) => (
          <li key={row.user?.id}>
            <Link
              to="/u/$handle"
              params={{ handle: row.user?.handle ?? "" }}
              className="flex items-center gap-3 rounded-2xl px-3 py-2.5 hover:bg-surface-2"
            >
              <Avatar user={row.user} />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-bold">
                  <EmojiText text={row.user?.displayName ?? ""} />
                </span>
                <ScopeChips scopes={row.scopes} className="mt-1" />
              </span>
              <span className="shrink-0 text-right text-xs text-ink-soft">
                {row.lastUsedAt ? <Ago at={row.lastUsedAt} /> : <When at={row.authorizedAt} />}
              </span>
            </Link>
          </li>
        ))}
      </ul>
      <LoadMore query={users} />
    </Card>
  );
}
