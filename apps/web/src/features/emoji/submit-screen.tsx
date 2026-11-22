import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { emoji as emojiApi } from "@tokuly/api";
import { BadgeCheck, Send } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { emojiKeys, emojiPageParam, shortcode, submissionClient, useEmojiDetail } from "./api";
import { syncCatalog } from "./catalog";
import { draftReady, EMPTY_DRAFT, type EmojiDraft, EmojiForm } from "./emoji-form";

/** The caller as a submitter: whether their emoji skip review, how many wait, and the limits. */
export function useSubmitterProfile() {
  return useQuery({
    queryKey: emojiKeys.submitter,
    queryFn: async () => (await submissionClient().getSubmitterProfile({})).profile ?? null,
  });
}

/** "Make an emoji": upload a picture, name it, describe it, send it to the moderators. */
export function SubmitScreen() {
  const { t } = useTranslation("emoji");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const profile = useSubmitterProfile();
  const [draft, setDraft] = useState<EmojiDraft>(EMPTY_DRAFT);
  const [uploading, setUploading] = useState(false);
  const limits = profile.data;
  const full = limits ? !limits.trusted && limits.pendingCount >= limits.maxPending : false;

  const submit = useMutation({
    mutationFn: () =>
      submissionClient().submitEmoji({
        name: draft.name,
        assetId: draft.assetId ?? "",
        category: draft.category,
        aliases: draft.aliases,
        license: draft.license.trim(),
        sensitive: draft.sensitive,
      }),
    onSuccess: async ({ emoji }) => {
      await queryClient.invalidateQueries({ queryKey: emojiKeys.submissions });
      await queryClient.invalidateQueries({ queryKey: emojiKeys.submitter });
      if (emoji?.state === emojiApi.EmojiState.APPROVED && emoji.emoji) {
        // A trusted submitter's emoji is in the catalogue already: bring this browser's copy along.
        await syncCatalog();
        toast.success(t("submit.added", { code: shortcode(emoji.emoji.name) }));
        await navigate({ to: "/emoji/$name", params: { name: emojiPageParam(emoji.emoji.name) } });
      } else {
        toast.success(t("submit.sent"));
        await navigate({ to: "/emoji/submissions" });
      }
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });

  return (
    <Page title={t("submit.title")} back="/emoji">
      {limits?.trusted ? (
        <p className="mb-4 flex items-center gap-2 rounded-2xl bg-lime px-4 py-3 text-sm font-bold text-[#17122b]">
          <BadgeCheck className="size-5 shrink-0" />
          {t("submit.trusted")}
        </p>
      ) : (
        <p className="mb-4 text-sm leading-relaxed text-ink-soft">
          {t("submit.lead")}
          {limits &&
            ` ${t("submit.pending", { count: limits.pendingCount, max: limits.maxPending })}`}
        </p>
      )}
      {full ? (
        <EmptyState
          emoji="🚦"
          title={t("submit.fullTitle")}
          body={t("submit.fullBody", { max: limits?.maxPending ?? 0 })}
          action={
            <Link
              to="/emoji/submissions"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("submissions.title")}
            </Link>
          }
        />
      ) : (
        <form
          className="rounded-card bg-surface p-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (draftReady(draft, uploading) && draft.assetId) submit.mutate();
          }}
        >
          <EmojiForm
            draft={draft}
            onChange={(patch) => setDraft((current) => ({ ...current, ...patch }))}
            onUploading={setUploading}
            maxImageBytes={limits ? Number(limits.maxImageBytes) : undefined}
            maxAliases={limits?.maxAliases || undefined}
          />
          <Button
            type="submit"
            block
            size="lg"
            className="mt-6"
            icon={<Send className="size-5" />}
            loading={submit.isPending}
            disabled={!draftReady(draft, uploading) || !draft.assetId}
          >
            {t(limits?.trusted ? "submit.sendTrusted" : "submit.send")}
          </Button>
          <p className="mt-3 text-center text-xs text-ink-faint">{t("submit.rules")}</p>
        </form>
      )}
    </Page>
  );
}

/** Changing a submission that waits or was declined; a declined one is submitted again by saving. */
export function EditSubmissionScreen({ emojiId }: { emojiId: string }) {
  const { t } = useTranslation("emoji");
  const detail = useEmojiDetail(emojiId);
  const data = detail.data;
  if (detail.isPending) {
    return (
      <Page title={t("submissions.editTitle")} back="/emoji/submissions">
        <Skeleton className="h-96" />
      </Page>
    );
  }
  if (!data?.emoji || !data.canEdit) {
    return (
      <Page title={t("submissions.editTitle")} back="/emoji/submissions">
        <EmptyState
          emoji="🔒"
          title={t("submissions.notEditable")}
          body={t("submissions.notEditableBody")}
        />
      </Page>
    );
  }
  return <EditForm detail={data} />;
}

function EditForm({ detail }: { detail: emojiApi.EmojiDetail }) {
  const { t } = useTranslation("emoji");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const profile = useSubmitterProfile();
  const current = detail.emoji as emojiApi.CustomEmoji;
  const rejected = detail.state === emojiApi.EmojiState.REJECTED;
  const [draft, setDraft] = useState<EmojiDraft>({
    name: current.name,
    pictureUrl: current.image?.url,
    category: current.category,
    aliases: [...current.aliases],
    license: detail.license,
    sensitive: current.sensitive,
    localOnly: current.localOnly,
  });
  const [uploading, setUploading] = useState(false);

  const save = useMutation({
    mutationFn: () =>
      submissionClient().updateSubmission({
        emojiId: current.id,
        name: draft.name,
        assetId: draft.assetId,
        category: draft.category,
        aliases: { aliases: draft.aliases },
        license: draft.license.trim(),
        sensitive: draft.sensitive,
      }),
    onSuccess: async () => {
      toast.success(t(rejected ? "submissions.resubmitted" : "submissions.saved"));
      await queryClient.invalidateQueries({ queryKey: emojiKeys.all });
      await navigate({ to: "/emoji/submissions" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });

  return (
    <Page title={t("submissions.editTitle")} back="/emoji/submissions">
      {rejected && (
        <div className="mb-4 rounded-2xl border-2 border-danger bg-surface px-4 py-3 text-sm">
          <p className="font-black text-danger">{t("submissions.rejectedTitle")}</p>
          {detail.reviewNote && (
            <p className="mt-1 whitespace-pre-wrap break-words">{detail.reviewNote}</p>
          )}
          <p className="mt-1 text-ink-soft">{t("submissions.rejectedBody")}</p>
        </div>
      )}
      <form
        className="rounded-card bg-surface p-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (draftReady(draft, uploading)) save.mutate();
        }}
      >
        <EmojiForm
          draft={draft}
          onChange={(patch) => setDraft((value) => ({ ...value, ...patch }))}
          onUploading={setUploading}
          // A declined emoji no longer holds its name: it is checked like a new one.
          currentName={rejected ? undefined : current.name}
          maxImageBytes={profile.data ? Number(profile.data.maxImageBytes) : undefined}
          maxAliases={profile.data?.maxAliases || undefined}
        />
        <Button
          type="submit"
          block
          size="lg"
          className="mt-6"
          loading={save.isPending}
          disabled={!draftReady(draft, uploading)}
        >
          {t(rejected ? "submissions.resubmit" : "submissions.save")}
        </Button>
      </form>
    </Page>
  );
}
