import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ask, common, toMillis } from "@tokuly/api";
import { askKeys, PAGE_SIZE } from "@tokuly/core/ask";
import { Flag, Lock, MoreHorizontal, ShieldBan, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { ReportButton, ReportDialog } from "#/features/engagement";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import {
  boxClient,
  promptClient,
  refreshPrompt,
  useBox,
  usePrompt,
  usePromptResponses,
} from "./api";
import { Picture, SectionTitle, Sender, toastError } from "./parts";
import { PromptState } from "./prompt-card";
import { SendForm } from "./send-form";

const FILTERS = {
  received: ask.ResponseFilter.RECEIVED,
  published: ask.ResponseFilter.PUBLISHED,
  held: ask.ResponseFilter.HELD,
} as const;
type FilterTab = keyof typeof FILTERS;

/** A prompt's page: its owner collects and publishes responses here, their friends respond and read what was published. */
export function PromptScreen({ promptId }: { promptId: string }) {
  const { t } = useTranslation("ask");
  const me = useMe();
  const { data, isPending, isError } = usePrompt(promptId);
  const prompt = data?.prompt;

  if (isPending) {
    return (
      <Page title={t("prompts.title")} back="/ask/prompts">
        <Skeleton className="h-32" />
        <Skeleton className="mt-4 h-40" />
      </Page>
    );
  }
  if (isError || !prompt?.owner) {
    return (
      <Page title={t("prompts.title")} back="/ask/prompts">
        <EmptyState
          emoji="🍃"
          title={t("prompts.page.notFound")}
          body={t("prompts.page.notFoundBody")}
        />
      </Page>
    );
  }
  return prompt.owner.id === me?.user?.id ? (
    <OwnerView prompt={prompt} />
  ) : (
    <FriendView prompt={prompt} myResponse={data?.myResponse} />
  );
}

function PromptHeader({ prompt }: { prompt: ask.Prompt }) {
  const { t } = useTranslation("ask");
  const owner = prompt.owner!;
  return (
    <div
      className="sticker rounded-card bg-sun px-5 pt-4 pb-5 text-[#17122b]"
      data-testid="prompt-header"
    >
      <div className="flex items-center gap-2">
        <Link
          to="/u/$handle"
          params={{ handle: owner.handle }}
          className="inline-flex min-w-0 items-center gap-1.5 text-sm font-black"
        >
          <Avatar user={owner} size="xs" outlined />
          <span className="truncate">{t("prompts.by", { name: owner.displayName })}</span>
        </Link>
        <span className="flex-1" />
        <PromptState prompt={prompt} className="border-2 border-ink bg-surface text-ink" />
      </div>
      <p className="mt-3 text-2xl leading-snug font-black break-words">
        {prompt.emoji && <span className="mr-2">{prompt.emoji}</span>}
        {prompt.text}
      </p>
    </div>
  );
}

// ───────────────────────────── The owner's side ─────────────────────────────

function OwnerView({ prompt }: { prompt: ask.Prompt }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [tab, setTab] = useState<FilterTab>("received");
  const [confirm, setConfirm] = useState<"close" | "delete" | null>(null);
  const query = usePromptResponses(prompt.id, FILTERS[tab], true);
  const responses = query.data?.pages.flatMap((page) => page.responses) ?? [];

  // Listing the received responses marks them as seen: the counters elsewhere follow.
  const listedAt = tab === "received" ? query.dataUpdatedAt : 0;
  useEffect(() => {
    if (!listedAt) return;
    void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
  }, [listedAt, queryClient]);

  const close = useMutation({
    mutationFn: () => promptClient().closePrompt({ promptId: prompt.id }),
    onSuccess: () => {
      refreshPrompt(queryClient, prompt.id);
      toast.success(t("prompts.page.closedToast"));
      setConfirm(null);
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => promptClient().deletePrompt({ promptId: prompt.id }),
    onSuccess: async () => {
      toast.success(t("prompts.page.deletedToast"));
      await navigate({ to: "/ask/prompts", replace: true });
      queryClient.removeQueries({ queryKey: askKeys.prompt(prompt.id) });
      void queryClient.invalidateQueries({ queryKey: askKeys.promptsAll });
    },
    onError: toastError,
  });

  const tabs: FilterTab[] =
    prompt.heldCount > 0 || tab === "held"
      ? ["received", "published", "held"]
      : ["received", "published"];
  const emptyKey =
    tab === "received" ? "emptyReceived" : tab === "published" ? "emptyPublished" : "emptyHeld";

  return (
    <Page
      title={t("prompts.title")}
      back="/ask/prompts"
      actions={
        <Menu
          trigger={
            <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            ...(prompt.open
              ? [
                  {
                    label: t("prompts.page.close"),
                    icon: <Lock />,
                    onSelect: () => setConfirm("close"),
                  },
                ]
              : []),
            {
              label: t("prompts.page.delete"),
              icon: <Trash2 />,
              danger: true,
              onSelect: () => setConfirm("delete"),
            },
          ]}
        />
      }
    >
      <PromptHeader prompt={prompt} />
      <p className="mt-3 px-1 text-xs leading-relaxed text-ink-soft">
        🔒 {t("prompts.page.ownerNote")}
      </p>

      <Tabs<FilterTab>
        className="mt-4"
        value={tab}
        onChange={setTab}
        items={tabs.map((value) => ({
          value,
          label: t(`prompts.page.tabs.${value}`),
          badge:
            value === "received"
              ? prompt.unseenCount
              : value === "held"
                ? prompt.heldCount
                : undefined,
        }))}
      />
      <div className="mt-4 flex flex-col gap-3" data-testid="responses">
        {query.isPending ? (
          <Skeleton className="h-24" />
        ) : responses.length === 0 ? (
          <EmptyState
            emoji={tab === "received" ? "📭" : tab === "published" ? "📌" : "🧹"}
            title={t(`prompts.page.${emptyKey}`)}
            body={tab === "held" ? undefined : t(`prompts.page.${emptyKey}Body`)}
          />
        ) : (
          <>
            {responses.map((response) => (
              <OwnerResponse key={response.id} response={response} />
            ))}
            <LoadMore query={query} />
          </>
        )}
      </div>

      <ConfirmDialog
        open={confirm === "close"}
        onOpenChange={() => setConfirm(null)}
        title={t("prompts.page.confirmClose.title")}
        description={t("prompts.page.confirmClose.body")}
        confirmLabel={t("prompts.page.confirmClose.confirm")}
        loading={close.isPending}
        onConfirm={() => close.mutate()}
      />
      <ConfirmDialog
        open={confirm === "delete"}
        onOpenChange={() => setConfirm(null)}
        title={t("prompts.page.confirmDelete.title")}
        description={t("prompts.page.confirmDelete.body")}
        confirmLabel={t("prompts.page.confirmDelete.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </Page>
  );
}

function OwnerResponse({ response }: { response: ask.PromptResponse }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const [dialog, setDialog] = useState<"delete" | "block" | "report" | null>(null);
  const refresh = () => refreshPrompt(queryClient, response.promptId);

  const publish = useMutation({
    mutationFn: (published: boolean) =>
      promptClient().setResponsePublished({ responseId: response.id, published }),
    onSuccess: refresh,
    onError: toastError,
  });
  const release = useMutation({
    mutationFn: () => promptClient().releaseResponse({ responseId: response.id }),
    onSuccess: refresh,
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => promptClient().deleteResponse({ responseId: response.id }),
    onSuccess: () => {
      refresh();
      toast.success(t("prompts.page.responseDeleted"));
      setDialog(null);
    },
    onError: toastError,
  });
  const block = useMutation({
    mutationFn: () =>
      boxClient().blockSender({ target: { case: "promptResponseId", value: response.id } }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: askKeys.blocks });
      toast.success(t("inbox.toast.blocked"));
      setDialog(null);
    },
    onError: toastError,
  });

  return (
    <article
      className="sticker rounded-card bg-surface px-4 pt-3.5 pb-3"
      data-testid="response-row"
    >
      <div className="flex items-center gap-2">
        <Sender anonymous={response.anonymous} user={response.responder} />
        {!response.seen && !response.held && (
          <span className="rounded-full bg-pink px-2 py-0.5 text-[11px] font-black text-white">
            {t("prompts.page.new")}
          </span>
        )}
        {response.published && (
          <span className="rounded-full bg-lime px-2 py-0.5 text-[11px] font-black text-[#17122b]">
            📌 {t("prompts.page.publishedBadge")}
          </span>
        )}
        <span className="flex-1" />
        <time className="text-xs font-bold text-ink-faint">
          {compactAge(toMillis(response.createdAt))}
        </time>
      </div>
      <p className="mt-2 text-lg leading-snug font-black break-words whitespace-pre-wrap">
        {response.text}
      </p>
      {response.image && <Picture image={response.image} className="mt-2 max-w-xs" />}
      <div className="mt-3 flex items-center gap-2">
        {response.held ? (
          <Button size="sm" loading={release.isPending} onClick={() => release.mutate()}>
            {t("prompts.page.release")}
          </Button>
        ) : (
          <Button
            size="sm"
            variant={response.published ? "secondary" : "lime"}
            loading={publish.isPending}
            onClick={() => publish.mutate(!response.published)}
          >
            {response.published ? t("prompts.page.unpublish") : t("prompts.page.publish")}
          </Button>
        )}
        <span className="flex-1" />
        <Menu
          trigger={
            <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
              <MoreHorizontal className="size-5" />
            </Button>
          }
          actions={[
            {
              label: t("inbox.actions.block"),
              icon: <ShieldBan />,
              onSelect: () => setDialog("block"),
            },
            {
              label: t("inbox.actions.report"),
              icon: <Flag />,
              onSelect: () => setDialog("report"),
            },
            {
              label: t("inbox.actions.delete"),
              icon: <Trash2 />,
              danger: true,
              onSelect: () => setDialog("delete"),
            },
          ]}
        />
      </div>
      <ConfirmDialog
        open={dialog === "delete"}
        onOpenChange={() => setDialog(null)}
        title={t("prompts.page.confirmDeleteResponse.title")}
        description={t("prompts.page.confirmDeleteResponse.body")}
        confirmLabel={t("prompts.page.confirmDeleteResponse.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <ConfirmDialog
        open={dialog === "block"}
        onOpenChange={() => setDialog(null)}
        title={t("inbox.confirmBlock.title")}
        description={t("inbox.confirmBlock.body")}
        confirmLabel={t("inbox.confirmBlock.confirm")}
        danger
        loading={block.isPending}
        onConfirm={() => block.mutate()}
      />
      <ReportDialog
        subject={{ type: common.SubjectType.QUESTION, id: response.id }}
        open={dialog === "report"}
        onOpenChange={(open) => !open && setDialog(null)}
        label={response.text}
      />
    </article>
  );
}

// ───────────────────────────── A friend's side ─────────────────────────────

function FriendView({
  prompt,
  myResponse,
}: {
  prompt: ask.Prompt;
  myResponse: ask.PromptResponse | undefined;
}) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const owner = prompt.owner!;
  // Whether the owner takes pictures is a setting of their box.
  const box = useBox(owner.id);
  const published = useInfiniteQuery({
    queryKey: [...askKeys.responsesAll(prompt.id), "page"],
    queryFn: ({ pageParam }) =>
      promptClient().listPublishedResponses({
        promptId: prompt.id,
        page: { cursor: pageParam, limit: PAGE_SIZE },
      }),
    initialPageParam: "",
    getNextPageParam: (page) => (page.page?.hasMore ? page.page.nextCursor : undefined),
  });
  const responses = published.data?.pages.flatMap((page) => page.responses) ?? [];

  return (
    <Page
      title={t("prompts.title")}
      back="/ask/prompts"
      actions={
        <ReportButton
          subject={{ type: common.SubjectType.QUESTION, id: prompt.id }}
          label={prompt.text}
        />
      }
    >
      <PromptHeader prompt={prompt} />

      {myResponse ? (
        <Card className="mt-5" data-testid="my-response">
          <p className="text-sm font-black text-ink-soft">
            {myResponse.anonymous ? "🙈" : "🙋"} {t("prompts.page.yourResponse")}
          </p>
          <p className="mt-1.5 text-lg leading-snug font-black break-words whitespace-pre-wrap">
            {myResponse.text}
          </p>
          {myResponse.image && <Picture image={myResponse.image} className="mt-2 max-w-xs" />}
          <p className="mt-2 text-xs text-ink-soft">
            {myResponse.published
              ? `📌 ${t("prompts.page.publishedBadge")}`
              : t("prompts.page.yourResponseNote", { name: owner.displayName })}
          </p>
        </Card>
      ) : prompt.canRespond ? (
        <Card className="mt-5">
          <SendForm
            recipientName={owner.displayName}
            allowSigned
            anonymousAvailable={prompt.anonymousAvailable}
            allowImages={box.data?.allowImages ?? false}
            placeholder={t("prompts.page.respondPlaceholder")}
            submitLabel={t("prompts.page.respondSubmit")}
            onSend={async (input) => {
              await promptClient().respondToPrompt({ promptId: prompt.id, ...input });
              toast.success(t("prompts.page.responded"));
              refreshPrompt(queryClient, prompt.id);
            }}
          />
        </Card>
      ) : (
        !prompt.open && (
          <p className="mt-5 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
            🔒 {t("prompts.page.closedNote")}
          </p>
        )
      )}

      <SectionTitle>{t("prompts.page.publishedTitle")}</SectionTitle>
      {published.isPending ? (
        <Skeleton className="h-24" />
      ) : responses.length === 0 ? (
        <p className="rounded-card bg-surface px-4 py-6 text-center text-sm font-bold text-ink-soft">
          🫧 {t("prompts.page.publishedEmpty")}
        </p>
      ) : (
        <div className="flex flex-col gap-3" data-testid="published-responses">
          {responses.map((response) => (
            <article key={response.id} className="sticker rounded-card bg-surface px-4 py-3.5">
              <Sender anonymous={response.anonymous} user={response.responder} />
              <p className="mt-1.5 text-lg leading-snug font-black break-words whitespace-pre-wrap">
                {response.text}
              </p>
              {response.image && <Picture image={response.image} className="mt-2 max-w-xs" />}
            </article>
          ))}
          <LoadMore query={published} />
        </div>
      )}
    </Page>
  );
}
