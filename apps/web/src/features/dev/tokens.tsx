import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { BarChart3, KeyRound, Plus, Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { dev, keys, nextCursorOf, PAGE_SIZE, tokenClient } from "./api";
import {
  Ago,
  Notice,
  type RevealedSecret,
  ScopeChips,
  SecretDialog,
  useSecretAction,
  When,
} from "./parts";
import { ScopePicker } from "./scope-picker";
import { UsagePanel } from "./usage";

const EXPIRIES = [
  dev.TokenExpiry.TOKEN_EXPIRY_7_DAYS,
  dev.TokenExpiry.TOKEN_EXPIRY_30_DAYS,
  dev.TokenExpiry.TOKEN_EXPIRY_90_DAYS,
  dev.TokenExpiry.TOKEN_EXPIRY_1_YEAR,
  dev.TokenExpiry.TOKEN_EXPIRY_NEVER,
] as const;

/** What the form of a new token collects. */
export interface TokenDraft {
  name: string;
  scopes: string[];
  expiry: dev.TokenExpiry;
}

/** The form of a new token (personal or a bot's): name, scopes, expiry. */
export function TokenFormDialog({
  open,
  onOpenChange,
  title,
  busy,
  onSubmit,
  offered,
  lead,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  lead?: string;
  busy: boolean;
  onSubmit(draft: TokenDraft): void;
  /** Leaves scopes out of the choice. */
  offered?(scope: dev.ScopeInfo): boolean;
}) {
  const { t } = useTranslation("dev");
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [expiry, setExpiry] = useState<dev.TokenExpiry>(dev.TokenExpiry.TOKEN_EXPIRY_30_DAYS);
  const ready = name.trim().length > 0 && scopes.length > 0;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (ready) onSubmit({ name: name.trim(), scopes, expiry });
  };
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      description={lead}
      footer={
        <Button type="submit" form="dev-token-form" loading={busy} disabled={!ready}>
          {t("tokens.create")}
        </Button>
      }
    >
      <form id="dev-token-form" onSubmit={submit} className="flex flex-col gap-4">
        <TextField
          label={t("tokens.name")}
          hint={t("tokens.nameHint")}
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={60}
          autoComplete="off"
          required
        />
        <SelectField
          label={t("tokens.expiry")}
          value={String(expiry)}
          onChange={(next) => setExpiry(Number(next) as dev.TokenExpiry)}
          options={EXPIRIES.map((value) => ({
            value: String(value),
            label: t(`tokens.expiries.${value}`),
          }))}
        />
        {expiry === dev.TokenExpiry.TOKEN_EXPIRY_NEVER && (
          <Notice tone="warning">{t("tokens.neverWarning")}</Notice>
        )}
        <div>
          <p className="mb-1.5 px-1 text-sm font-bold text-ink-soft">{t("tokens.scopes")}</p>
          <ScopePicker value={scopes} onChange={setScopes} offered={offered} />
        </div>
      </form>
    </Dialog>
  );
}

/** One token of a list: what it is, what it may do, when it was used, and how to end it. */
export function TokenCard({
  token,
  onRevoke,
  revoking,
  usage,
}: {
  token: dev.AccessToken;
  onRevoke(): void;
  revoking: boolean;
  /** Offer the usage statistics of the token (personal tokens only). */
  usage?: boolean;
}) {
  const { t } = useTranslation("dev");
  const [confirming, setConfirming] = useState(false);
  const [showUsage, setShowUsage] = useState(false);
  return (
    <Card className="p-4">
      <div className="flex items-start gap-3">
        <span className="grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink bg-lime text-[#17122b]">
          <KeyRound className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 font-black">
            <span className="min-w-0 break-words">
              <EmojiText text={token.name} />
            </span>
            {token.expired && (
              <Chip className="bg-danger/15 text-danger">{t("tokens.expired")}</Chip>
            )}
          </p>
          <p className="font-mono text-xs text-ink-soft">{token.hint}</p>
        </div>
      </div>
      <ScopeChips scopes={token.scopes} className="mt-3" />
      <dl className="mt-3 grid gap-x-4 gap-y-1 text-sm sm:grid-cols-3">
        <div>
          <dt className="text-xs font-bold text-ink-faint">{t("tokens.created")}</dt>
          <dd>
            <When at={token.createdAt} />
          </dd>
        </div>
        <div>
          <dt className="text-xs font-bold text-ink-faint">{t("tokens.expires")}</dt>
          <dd>{token.expiresAt ? <When at={token.expiresAt} /> : t("tokens.noExpiry")}</dd>
        </div>
        <div>
          <dt className="text-xs font-bold text-ink-faint">{t("tokens.lastUsed")}</dt>
          <dd>
            {token.lastUsedAt ? (
              <>
                <Ago at={token.lastUsedAt} />
                {token.lastUsedIp && (
                  <span className="ml-1.5 font-mono text-xs text-ink-soft">{token.lastUsedIp}</span>
                )}
              </>
            ) : (
              t("tokens.neverUsed")
            )}
          </dd>
        </div>
      </dl>
      <div className="mt-3 flex flex-wrap justify-end gap-2">
        {usage && (
          <Button
            variant="ghost"
            size="sm"
            icon={<BarChart3 className="size-4" />}
            aria-expanded={showUsage}
            onClick={() => setShowUsage((shown) => !shown)}
          >
            {t("usage.title")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-danger"
          icon={<Trash2 className="size-4" />}
          onClick={() => setConfirming(true)}
        >
          {t("tokens.revoke")}
        </Button>
      </div>
      {showUsage && (
        <div className="mt-3 border-t-2 border-line pt-3">
          <UsagePanel subject={{ kind: "token", id: token.id }} />
        </div>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("tokens.revokeTitle", { name: token.name })}
        description={t("tokens.revokeBody")}
        confirmLabel={t("tokens.revoke")}
        danger
        loading={revoking}
        onConfirm={onRevoke}
      />
    </Card>
  );
}

/** `/developers/tokens`: the caller's personal access tokens. */
export function TokensScreen() {
  const { t } = useTranslation("dev");
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const [revealed, setRevealed] = useState<RevealedSecret | null>(null);

  const tokens = useInfiniteQuery({
    queryKey: keys.tokens,
    queryFn: ({ pageParam }) =>
      tokenClient().listTokens({ page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: nextCursorOf,
  });

  const create = useSecretAction(
    (draft: TokenDraft) => tokenClient().createToken(draft),
    (response) => {
      setCreating(false);
      setRevealed({
        title: t("tokens.createdTitle"),
        lead: t("tokens.createdLead"),
        secret: response.secret,
      });
      void queryClient.invalidateQueries({ queryKey: keys.tokens });
    },
  );

  const revoke = useMutation({
    mutationFn: (tokenId: string) => tokenClient().revokeToken({ tokenId }),
    onSuccess: () => {
      toast.success(t("tokens.revoked"));
      return queryClient.invalidateQueries({ queryKey: keys.tokens });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "dev" })),
  });

  const rows = tokens.data?.pages.flatMap((page) => page.tokens) ?? [];
  return (
    <Page
      title={t("tokens.title")}
      back="/developers"
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("tokens.new")}
        </Button>
      }
    >
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("tokens.lead")}</p>
      {tokens.isPending ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-40" />
          <Skeleton className="h-40" />
        </div>
      ) : tokens.isError ? (
        <Notice tone="danger">{errorMessage(tokens.error)}</Notice>
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🔑"
          title={t("tokens.empty")}
          body={t("tokens.emptyBody")}
          action={<Button onClick={() => setCreating(true)}>{t("tokens.new")}</Button>}
        />
      ) : (
        <ul className="flex flex-col gap-3">
          {rows.map((token) => (
            <li key={token.id}>
              <TokenCard
                token={token}
                usage
                revoking={revoke.isPending && revoke.variables === token.id}
                onRevoke={() => revoke.mutate(token.id)}
              />
            </li>
          ))}
          <LoadMore query={tokens} />
        </ul>
      )}

      {/* Mounted only while open, so every new token starts from an empty form. */}
      {creating && (
        <TokenFormDialog
          open
          onOpenChange={setCreating}
          title={t("tokens.newTitle")}
          busy={create.busy}
          onSubmit={create.run}
        />
      )}
      <SecretDialog revealed={revealed} onClose={() => setRevealed(null)} />
    </Page>
  );
}
