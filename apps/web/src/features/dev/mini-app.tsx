import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { errorReason, media } from "@tokuly/api";
import { miniAppHref, miniAppKeys, miniAppQueries } from "@tokuly/core/miniapps";
import { ExternalLink, Send, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { ShareToChat } from "#/features/chat";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { dev, localized, miniAppClient } from "./api";
import { ImageField } from "./image-field";
import { Fact, Notice, Section, When } from "./parts";

interface ManifestDraft {
  name: string;
  launchUrl: string;
  scopes: string[];
  /** `undefined` keeps the saved icon, `""` removes it. */
  iconAssetId: string | undefined;
}

const draftOf = (manifest: dev.MiniAppManifest | undefined, appName: string): ManifestDraft => ({
  name: manifest?.name ?? appName.slice(0, 30),
  launchUrl: manifest?.launchUrl ?? "",
  scopes: manifest?.scopes ?? [],
  iconAssetId: undefined,
});

const STATE_TONE: Record<dev.MiniAppReviewState, string> = {
  [dev.MiniAppReviewState.UNSPECIFIED]: "",
  [dev.MiniAppReviewState.DRAFT]: "",
  [dev.MiniAppReviewState.IN_REVIEW]: "bg-sun text-[#17122b]",
  [dev.MiniAppReviewState.APPROVED]: "bg-lime text-[#17122b]",
  [dev.MiniAppReviewState.REJECTED]: "bg-danger/10 text-danger",
};

/**
 * The mini app of an application (`docs/miniapps.md`): its manifest, the
 * review it is in, and once approved a way to open it and share it to a chat.
 */
export function MiniAppSection({ app }: { app: dev.App }) {
  const { t } = useTranslation("dev");
  const review = useQuery({
    queryKey: miniAppKeys.review(app.id),
    queryFn: async () => {
      try {
        return (await miniAppClient().getMiniAppReview({ appId: app.id })).review ?? null;
      } catch (error) {
        // No manifest yet: the form starts empty.
        if (errorReason(error) === "MINI_APP_NOT_FOUND") return null;
        throw error;
      }
    },
  });
  return (
    <Section title={t("miniApp.title")}>
      <Card className="flex flex-col gap-4 p-4">
        <p className="text-sm leading-relaxed text-ink-soft">{t("miniApp.lead")}</p>
        {review.isPending ? (
          <Skeleton className="h-64" />
        ) : review.isError ? (
          <Notice tone="danger">{errorMessage(review.error, { namespace: "dev" })}</Notice>
        ) : (
          <MiniAppEditor
            key={review.data?.revision ?? 0}
            app={app}
            review={review.data ?? undefined}
          />
        )}
      </Card>
    </Section>
  );
}

function MiniAppEditor({ app, review }: { app: dev.App; review: dev.MiniAppReview | undefined }) {
  const { t } = useTranslation("dev");
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(review?.draft, app.name));
  const [sharing, setSharing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const scopes = useQuery(miniAppQueries.scopes(miniAppClient));
  const set = <K extends keyof ManifestDraft>(key: K, value: ManifestDraft[K]) =>
    setDraft({ ...draft, [key]: value });
  const store = (next: dev.MiniAppReview | undefined | null) =>
    queryClient.setQueryData(miniAppKeys.review(app.id), next ?? null);
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "dev" }));

  const save = useMutation({
    mutationFn: () =>
      miniAppClient().setMiniAppManifest({
        appId: app.id,
        name: draft.name.trim(),
        launchUrl: draft.launchUrl.trim(),
        scopes: draft.scopes,
        iconAssetId: draft.iconAssetId,
      }),
    onSuccess: (response) => {
      store(response.review);
      toast.success(t("miniApp.saved"));
    },
    onError,
  });
  const submit = useMutation({
    mutationFn: () => miniAppClient().submitMiniApp({ appId: app.id }),
    onSuccess: (response) => {
      store(response.review);
      toast.success(t("miniApp.submitted"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => miniAppClient().deleteMiniApp({ appId: app.id }),
    onSuccess: () => {
      store(null);
      setDeleting(false);
      toast.success(t("miniApp.deleted"));
    },
    onError,
  });

  const state = review?.state ?? dev.MiniAppReviewState.DRAFT;
  const live = review?.live;
  const liveOrigin = live ? new URL(live.launchUrl).origin : "";
  const ready = draft.name.trim().length > 0 && draft.launchUrl.trim().length > 0;
  const toggle = (scope: string) =>
    set(
      "scopes",
      draft.scopes.includes(scope)
        ? draft.scopes.filter((chosen) => chosen !== scope)
        : [...draft.scopes, scope],
    );

  return (
    <>
      {review && (
        <div className="flex flex-col gap-2">
          <p className="flex flex-wrap items-center gap-2 text-sm font-bold">
            <Chip className={cn(STATE_TONE[state])} data-testid="mini-app-state">
              {t(`miniApp.states.${state}`)}
            </Chip>
            {review.submittedAt && state === dev.MiniAppReviewState.IN_REVIEW && (
              <span className="text-xs text-ink-soft">
                {t("miniApp.submittedAt")} <When at={review.submittedAt} />
              </span>
            )}
          </p>
          {state === dev.MiniAppReviewState.REJECTED && (
            <Notice tone="danger">{t("miniApp.rejected", { note: review.reviewNote })}</Notice>
          )}
        </div>
      )}
      {live && (
        <div className="rounded-2xl border-2 border-line p-3">
          <dl>
            <Fact label={t("miniApp.live")}>
              <span className="break-all font-mono text-xs">{live.launchUrl}</span>
            </Fact>
            {review?.approvedAt && (
              <Fact label={t("miniApp.approvedAt")}>
                <When at={review.approvedAt} />
              </Fact>
            )}
          </dl>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" icon={<Send className="size-4" />} onClick={() => setSharing(true)}>
              {t("miniApp.share")}
            </Button>
            <a
              href={miniAppHref(app.id, { origin: liveOrigin })}
              className="pressable sticker inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
            >
              <ExternalLink className="size-4" />
              {t("miniApp.open")}
            </a>
          </div>
        </div>
      )}

      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) save.mutate();
        }}
      >
        <ImageField
          label={t("miniApp.icon")}
          name={draft.name}
          current={review?.draft?.icon}
          purpose={media.AssetPurpose.CHANNEL_ICON}
          value={draft.iconAssetId}
          onChange={(assetId) => set("iconAssetId", assetId)}
        />
        <TextField
          label={t("miniApp.name")}
          hint={t("miniApp.nameHint")}
          value={draft.name}
          onChange={(event) => set("name", event.target.value)}
          maxLength={30}
          autoComplete="off"
          required
        />
        <TextField
          label={t("miniApp.launchUrl")}
          hint={t("miniApp.launchUrlHint")}
          value={draft.launchUrl}
          onChange={(event) => set("launchUrl", event.target.value)}
          placeholder="https://"
          inputMode="url"
          autoCapitalize="none"
          spellCheck={false}
          required
        />
        <fieldset className="flex flex-col gap-2">
          <legend className="mb-1.5 px-1 text-sm font-bold text-ink-soft">
            {t("miniApp.scopes")}
          </legend>
          {(scopes.data ?? []).map((scope) => (
            <label
              key={scope.scope}
              className="flex items-start gap-3 rounded-2xl border-2 border-line px-3 py-2.5"
            >
              <input
                type="checkbox"
                checked={draft.scopes.includes(scope.scope)}
                onChange={() => toggle(scope.scope)}
                className="mt-1 size-4 accent-brand"
              />
              <span className="min-w-0">
                <span className="block font-mono text-xs font-bold">{scope.scope}</span>
                <span className="block text-sm text-ink-soft">{localized(scope.description)}</span>
              </span>
            </label>
          ))}
        </fieldset>
        <p className="px-1 text-xs leading-relaxed text-ink-faint">{t("miniApp.reviewHint")}</p>
        <div className="flex flex-wrap gap-2">
          <Button type="submit" loading={save.isPending} disabled={!ready}>
            {t("miniApp.save")}
          </Button>
          {review && state === dev.MiniAppReviewState.DRAFT && (
            <Button
              type="button"
              variant="secondary"
              loading={submit.isPending}
              onClick={() => submit.mutate()}
            >
              {t("miniApp.submit")}
            </Button>
          )}
          {review && (
            <Button
              type="button"
              variant="ghost"
              icon={<Trash2 className="size-4" />}
              onClick={() => setDeleting(true)}
            >
              {t("miniApp.delete")}
            </Button>
          )}
        </div>
      </form>

      {live && (
        <ShareToChat
          open={sharing}
          onOpenChange={setSharing}
          share={{ url: `${window.location.origin}${miniAppHref(app.id)}` }}
          title={live.name}
        />
      )}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("miniApp.deleteTitle")}
        description={t("miniApp.deleteLead")}
        confirmLabel={t("miniApp.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}
