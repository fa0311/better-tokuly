import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { admin, type common, fromDate, isApiError, isNotFound, media } from "@tokuly/api";
import { CalendarClock, ExternalLink, ImagePlus, Save, Send, Trash2, Undo2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import {
  ANNOUNCEMENT_CATEGORIES,
  AnnouncementArticle,
  AnnouncementCard,
  type AnnouncementContent,
  announcementCategoryKey,
  markdownExcerpt,
} from "#/features/announcements";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { announcementAdminClient } from "../api";
import { formatDateTime, fromLocalInput } from "../format";
import { ConsolePage, Fact, Section } from "../ui/console-page";
import { FilterInput, FilterSelect } from "../ui/filters";
import { ErrorState } from "../ui/states";
import { UserCell } from "../ui/user";
import { AnnouncementStatusChip } from "./list-screen";

// Limits of the API, per language.
const TITLE_MAX = 120;
const SUMMARY_MAX = 300;
const BODY_MAX = 20_000;

type Language = "ja" | "en";
type Localized = Record<Language, string>;

/** What the form edits. Japanese is required; English falls back to Japanese when left empty. */
interface Draft {
  category: admin.AnnouncementCategory;
  pinned: boolean;
  title: Localized;
  summary: Localized;
  body: Localized;
  image?: common.Image;
}

const EMPTY: Draft = {
  category: admin.AnnouncementCategory.UPDATE,
  pinned: false,
  title: { ja: "", en: "" },
  summary: { ja: "", en: "" },
  body: { ja: "", en: "" },
};

const localized = (text: admin.LocalizedText | undefined): Localized => ({
  ja: text?.ja ?? "",
  en: text?.en ?? "",
});

function draftOf(announcement: admin.ManagedAnnouncement): Draft {
  return {
    category: announcement.category || admin.AnnouncementCategory.UPDATE,
    pinned: announcement.pinned,
    title: localized(announcement.title),
    summary: localized(announcement.summary),
    body: localized(announcement.body),
    image: announcement.image?.url ? announcement.image : undefined,
  };
}

/** Whether two drafts would save the same thing. */
function sameDraft(a: Draft, b: Draft): boolean {
  const comparable = (draft: Draft) =>
    JSON.stringify([
      draft.category,
      draft.pinned,
      draft.title,
      draft.summary,
      draft.body,
      draft.image?.url ?? "",
    ]);
  return comparable(a) === comparable(b);
}

/** `/admin/announcements/new` and `/admin/announcements/:announcementId`. */
export function AnnouncementEditorScreen({ announcementId }: { announcementId?: string }) {
  const { t } = useTranslation("admin");
  const detail = useQuery({
    queryKey: ["admin", "announcements", "managed", "detail", announcementId],
    queryFn: () => announcementAdminClient().getManagedAnnouncement({ id: announcementId }),
    enabled: Boolean(announcementId),
  });
  if (!announcementId) return <Editor />;
  if (detail.isPending) {
    return (
      <ConsolePage title={t("announcements.title")} back="/admin/announcements">
        <div className="flex flex-col gap-4" aria-busy>
          <Skeleton className="h-24" />
          <Skeleton className="h-96" />
        </div>
      </ConsolePage>
    );
  }
  const announcement = detail.data?.announcement;
  if (!announcement) {
    return (
      <ConsolePage title={t("announcements.title")} back="/admin/announcements">
        {detail.isError && !isNotFound(detail.error) ? (
          <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
        ) : (
          <EmptyState emoji="🧭" title={t("announcements.notFound")} />
        )}
      </ConsolePage>
    );
  }
  return <Editor key={announcement.id} announcement={announcement} />;
}

function Editor({ announcement }: { announcement?: admin.ManagedAnnouncement }) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<Draft>(announcement ? draftOf(announcement) : EMPTY);
  const [showErrors, setShowErrors] = useState(false);
  const [previewLanguage, setPreviewLanguage] = useState<Language>("ja");
  const dirty = !sameDraft(draft, announcement ? draftOf(announcement) : EMPTY);
  const set = (changes: Partial<Draft>) => setDraft((current) => ({ ...current, ...changes }));

  /** Puts the server's new version into the cache and re-reads the lists that show it. */
  const stored = (next: admin.ManagedAnnouncement | undefined) => {
    if (next)
      queryClient.setQueryData<admin.GetManagedAnnouncementResponse>(
        ["admin", "announcements", "managed", "detail", next.id],
        (previous) => ({
          ...(previous as admin.GetManagedAnnouncementResponse),
          announcement: next,
        }),
      );
    return queryClient.invalidateQueries({ queryKey: ["admin"] });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "admin" }));

  const save = useMutation({
    mutationFn: async () => {
      const texts = { title: draft.title, summary: draft.summary, body: draft.body };
      if (!announcement)
        return (
          await announcementAdminClient().createAnnouncement({
            ...texts,
            category: draft.category,
            pinned: draft.pinned,
            image: draft.image,
          })
        ).announcement;
      const imageChanged = (draft.image?.url ?? "") !== (announcement.image?.url ?? "");
      return (
        await announcementAdminClient().updateAnnouncement({
          id: announcement.id,
          ...texts,
          category: draft.category,
          pinned: draft.pinned,
          image: imageChanged ? draft.image : undefined,
          clearImage: imageChanged && !draft.image,
        })
      ).announcement;
    },
    onSuccess: async (next) => {
      toast.success(t(announcement ? "announcements.toast.saved" : "announcements.toast.created"));
      await stored(next);
      if (!next) return;
      // The server trims texts: carry on from exactly what it stored.
      setDraft(draftOf(next));
      if (!announcement)
        await navigate({
          to: "/admin/announcements/$announcementId",
          params: { announcementId: next.id },
          replace: true,
        });
    },
    onError,
  });

  const missingTitle = !draft.title.ja.trim();
  const missingBody = !draft.body.ja.trim();
  const submit = () => {
    setShowErrors(true);
    if (!missingTitle && !missingBody) save.mutate();
  };

  const preview: AnnouncementContent = {
    category: draft.category,
    pinned: draft.pinned,
    image: draft.image,
    title: draft.title[previewLanguage] || draft.title.ja || t("announcements.untitled"),
    summary:
      draft.summary[previewLanguage] ||
      draft.summary.ja ||
      markdownExcerpt(draft.body[previewLanguage] || draft.body.ja, 120),
    body: draft.body[previewLanguage] || draft.body.ja,
    publishedAt: announcement?.publishedAt,
  };

  return (
    <ConsolePage
      title={
        announcement ? draft.title.ja || t("announcements.untitled") : t("announcements.create")
      }
      back="/admin/announcements"
      actions={
        <>
          {announcement && <AnnouncementStatusChip status={announcement.status} />}
          <Button
            size="sm"
            icon={<Save className="size-4" />}
            disabled={Boolean(announcement) && !dirty}
            loading={save.isPending}
            onClick={submit}
          >
            {announcement ? t("announcements.save") : t("announcements.saveDraft")}
          </Button>
        </>
      }
    >
      <div className="@container">
        <div className="grid items-start gap-4 @5xl:grid-cols-[minmax(0,1fr)_24rem]">
          <div className="flex min-w-0 flex-col gap-4">
            <Section title={t("announcements.sections.settings")}>
              <div className="grid gap-4 p-4 @2xl:grid-cols-2">
                <div className="flex flex-col gap-2">
                  <FilterSelect
                    label={t("announcements.fields.category")}
                    value={String(draft.category)}
                    onChange={(value) =>
                      set({ category: Number(value) as admin.AnnouncementCategory })
                    }
                    options={ANNOUNCEMENT_CATEGORIES.map((category) => ({
                      value: String(category),
                      label: t(`announcements:${announcementCategoryKey(category)}`),
                    }))}
                  />
                  <div className="-mx-3">
                    <SwitchRow
                      label={t("announcements.fields.pinned")}
                      hint={t("announcements.fields.pinnedHint")}
                      checked={draft.pinned}
                      onChange={(pinned) => set({ pinned })}
                    />
                  </div>
                </div>
                <HeaderImage image={draft.image} onChange={(image) => set({ image })} />
              </div>
            </Section>

            <div className="grid gap-4 @3xl:grid-cols-2">
              {(["ja", "en"] as const).map((language) => (
                <Section
                  key={language}
                  title={t(`announcements.languages.${language}`)}
                  aside={
                    <span className="text-xs text-ink-faint">
                      {t(language === "ja" ? "announcements.required" : "announcements.optional")}
                    </span>
                  }
                >
                  <div className="flex flex-col gap-4 p-4">
                    <TextField
                      label={t("announcements.fields.title")}
                      value={draft.title[language]}
                      maxLength={TITLE_MAX}
                      error={
                        language === "ja" && showErrors && missingTitle
                          ? t("errors.TITLE_INVALID")
                          : undefined
                      }
                      onChange={(event) =>
                        set({ title: { ...draft.title, [language]: event.target.value } })
                      }
                      lang={language}
                    />
                    <TextArea
                      label={t("announcements.fields.summary")}
                      hint={t("announcements.fields.summaryHint")}
                      value={draft.summary[language]}
                      maxLength={SUMMARY_MAX}
                      rows={2}
                      onChange={(event) =>
                        set({ summary: { ...draft.summary, [language]: event.target.value } })
                      }
                      lang={language}
                    />
                    <TextArea
                      label={t("announcements.fields.body")}
                      hint={t("announcements.fields.bodyHint")}
                      value={draft.body[language]}
                      maxLength={BODY_MAX}
                      rows={14}
                      error={
                        language === "ja" && showErrors && missingBody
                          ? t("errors:BODY_INVALID")
                          : undefined
                      }
                      onChange={(event) =>
                        set({ body: { ...draft.body, [language]: event.target.value } })
                      }
                      lang={language}
                    />
                  </div>
                </Section>
              ))}
            </div>
          </div>

          {/* Beside the form when there is room for a third column; otherwise under it, publishing next to the preview. */}
          <div className="grid min-w-0 items-start gap-4 @3xl:grid-cols-[20rem_minmax(0,1fr)] @5xl:grid-cols-1">
            {announcement ? (
              <PublishPanel announcement={announcement} dirty={dirty} onStored={stored} />
            ) : (
              <p className="rounded-2xl border-2 border-dashed border-line px-4 py-3 text-sm text-ink-soft">
                {t("announcements.publish.saveFirst")}
              </p>
            )}
            <Section
              title={t("announcements.preview.title")}
              aside={
                <Tabs<Language>
                  value={previewLanguage}
                  onChange={setPreviewLanguage}
                  items={[
                    { value: "ja", label: "日本語" },
                    { value: "en", label: "English" },
                  ]}
                />
              }
            >
              <div className="flex flex-col gap-4 bg-bg p-4" lang={previewLanguage}>
                <p className="text-xs font-bold text-ink-faint">
                  {t("announcements.preview.card")}
                </p>
                <AnnouncementCard announcement={preview} className="h-auto" />
                <p className="text-xs font-bold text-ink-faint">
                  {t("announcements.preview.article")}
                </p>
                <AnnouncementArticle announcement={preview} />
              </div>
            </Section>
          </div>
        </div>
      </div>
    </ConsolePage>
  );
}

/** The optional header image: uploaded through the media service, attached as its `Image`. */
function HeaderImage({
  image,
  onChange,
}: {
  image?: common.Image;
  onChange(image: common.Image | undefined): void;
}) {
  const { t } = useTranslation("admin");
  const input = useRef<HTMLInputElement>(null);
  const [progress, setProgress] = useState<number | null>(null);

  const upload = async (file: File) => {
    setProgress(0);
    try {
      const asset = await uploadAndProcess(file, {
        purpose: media.AssetPurpose.THUMBNAIL,
        onProgress: setProgress,
      });
      if (!asset.image?.url) throw new Error("the upload is not an image");
      onChange(asset.image);
    } catch (error) {
      toast.error(
        isApiError(error)
          ? errorMessage(error, { namespace: "admin" })
          : t("announcements.image.failed"),
      );
    } finally {
      setProgress(null);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <p className="px-1 text-[11px] font-bold text-ink-faint">{t("announcements.image.label")}</p>
      {image?.url ? (
        <img
          src={image.url}
          alt=""
          className="aspect-[16/7] w-full rounded-xl border-2 border-line object-cover"
        />
      ) : (
        <div className="grid aspect-[16/7] w-full place-items-center rounded-xl border-2 border-dashed border-line text-xs text-ink-faint">
          {t("announcements.image.none")}
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <input
          ref={input}
          type="file"
          accept="image/jpeg,image/png,image/webp,image/gif"
          className="hidden"
          aria-label={t("announcements.image.choose")}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) void upload(file);
          }}
        />
        <Button
          size="sm"
          variant="secondary"
          icon={<ImagePlus className="size-4" />}
          loading={progress !== null}
          onClick={() => input.current?.click()}
        >
          {progress !== null
            ? t("announcements.image.uploading", { percent: Math.round(progress * 100) })
            : t(image ? "announcements.image.replace" : "announcements.image.choose")}
        </Button>
        {image && progress === null && (
          <Button
            size="sm"
            variant="ghost"
            icon={<Trash2 className="size-4" />}
            onClick={() => onChange(undefined)}
          >
            {t("announcements.image.remove")}
          </Button>
        )}
      </div>
    </div>
  );
}

/** Where an announcement stands and the ways to change that: publish, schedule, take back, delete. */
function PublishPanel({
  announcement,
  dirty,
  onStored,
}: {
  announcement: admin.ManagedAnnouncement;
  dirty: boolean;
  onStored(next: admin.ManagedAnnouncement | undefined): Promise<void>;
}) {
  const { t } = useTranslation("admin");
  const navigate = useNavigate();
  const [when, setWhen] = useState("");
  const [notify, setNotify] = useState(announcement.notifyUsers);
  const [confirming, setConfirming] = useState<"publish" | "unpublish" | "delete" | null>(null);
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "admin" }));

  const scheduledFor = fromLocalInput(when);
  const later = scheduledFor !== undefined && scheduledFor.getTime() > Date.now();
  const isDraft = announcement.status === admin.AnnouncementStatus.DRAFT;
  const isScheduled = announcement.status === admin.AnnouncementStatus.SCHEDULED;
  const isPublished = announcement.status === admin.AnnouncementStatus.PUBLISHED;
  const alreadyNotified = Boolean(announcement.notifiedAt);

  const publish = useMutation({
    mutationFn: () =>
      announcementAdminClient().publishAnnouncement({
        id: announcement.id,
        publishAt: later && scheduledFor ? fromDate(scheduledFor) : undefined,
        notifyUsers: notify && !alreadyNotified,
      }),
    onSuccess: (response) => {
      toast.success(t(later ? "announcements.toast.scheduled" : "announcements.toast.published"));
      setConfirming(null);
      setWhen("");
      return onStored(response.announcement);
    },
    onError,
  });
  const unpublish = useMutation({
    mutationFn: () => announcementAdminClient().unpublishAnnouncement({ id: announcement.id }),
    onSuccess: (response) => {
      toast.success(t("announcements.toast.unpublished"));
      setConfirming(null);
      return onStored(response.announcement);
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: () => announcementAdminClient().deleteAnnouncement({ id: announcement.id }),
    onSuccess: async () => {
      toast.success(t("announcements.toast.deleted"));
      setConfirming(null);
      await navigate({ to: "/admin/announcements" });
      await onStored(undefined);
    },
    onError,
  });

  return (
    <Section
      title={t("announcements.publish.title")}
      aside={<AnnouncementStatusChip status={announcement.status} />}
    >
      <dl className="divide-y divide-line">
        {announcement.publishedAt && (
          <Fact
            label={t(
              isScheduled
                ? "announcements.publish.scheduledFor"
                : "announcements.columns.publishedAt",
            )}
          >
            {formatDateTime(announcement.publishedAt)}
          </Fact>
        )}
        {announcement.notifyUsers && (
          <Fact label={t("announcements.columns.notify")}>
            {alreadyNotified
              ? t("announcements.notifiedCount", { count: announcement.notifiedCount })
              : t("announcements.notifyPending")}
          </Fact>
        )}
        <Fact label={t("announcements.columns.author")}>
          <UserCell user={announcement.author} />
        </Fact>
        <Fact label={t("announcements.columns.updated")}>
          {formatDateTime(announcement.updatedAt)}
        </Fact>
      </dl>

      {!isPublished && (
        <div className="flex flex-col gap-3 border-t-2 border-line p-4">
          <FilterInput
            type="datetime-local"
            label={t("announcements.publish.when")}
            value={when}
            onChange={setWhen}
          />
          <p className="-mt-1 px-1 text-xs text-ink-faint">{t("announcements.publish.whenHint")}</p>
          <div className="-mx-3">
            <SwitchRow
              label={t("announcements.publish.notify")}
              hint={t(
                alreadyNotified
                  ? "announcements.publish.notifyDone"
                  : "announcements.publish.notifyHint",
              )}
              checked={notify && !alreadyNotified}
              disabled={alreadyNotified}
              onChange={setNotify}
            />
          </div>
          <Button
            block
            variant="lime"
            icon={later ? <CalendarClock className="size-4" /> : <Send className="size-4" />}
            disabled={dirty}
            onClick={() => setConfirming("publish")}
          >
            {t(
              later
                ? isScheduled
                  ? "announcements.publish.reschedule"
                  : "announcements.publish.schedule"
                : "announcements.publish.now",
            )}
          </Button>
          {dirty && (
            <p className="px-1 text-xs font-bold text-ink-soft">
              {t("announcements.publish.dirtyHint")}
            </p>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2 border-t-2 border-line p-4">
        {isPublished && (
          <Link
            to="/announcements/$announcementId"
            params={{ announcementId: announcement.id }}
            className="inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-bold text-brand hover:bg-surface-2"
          >
            <ExternalLink className="size-4" />
            {t("announcements.publish.view")}
          </Link>
        )}
        {!isDraft && (
          <Button
            size="sm"
            variant="secondary"
            icon={<Undo2 className="size-4" />}
            onClick={() => setConfirming("unpublish")}
          >
            {t(
              isScheduled
                ? "announcements.unpublish.openScheduled"
                : "announcements.unpublish.open",
            )}
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          className="text-danger"
          icon={<Trash2 className="size-4" />}
          onClick={() => setConfirming("delete")}
        >
          {t("announcements.delete.open")}
        </Button>
      </div>

      <ConfirmDialog
        open={confirming === "publish"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t(
          later
            ? "announcements.publish.confirmScheduleTitle"
            : "announcements.publish.confirmTitle",
        )}
        description={
          later && scheduledFor
            ? t("announcements.publish.confirmScheduleBody", { time: formatDateTime(scheduledFor) })
            : t("announcements.publish.confirmBody")
        }
        confirmLabel={t(later ? "announcements.publish.schedule" : "announcements.publish.now")}
        loading={publish.isPending}
        onConfirm={() => publish.mutate()}
      >
        {notify && !alreadyNotified && (
          <p className="rounded-xl bg-sun px-3 py-2 text-sm font-bold text-[#17122b]">
            {t("announcements.publish.confirmNotify")}
          </p>
        )}
      </ConfirmDialog>
      <ConfirmDialog
        open={confirming === "unpublish"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("announcements.unpublish.title")}
        description={t(
          isScheduled ? "announcements.unpublish.bodyScheduled" : "announcements.unpublish.body",
        )}
        confirmLabel={t("announcements.unpublish.confirm")}
        danger
        loading={unpublish.isPending}
        onConfirm={() => unpublish.mutate()}
      />
      <ConfirmDialog
        open={confirming === "delete"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("announcements.delete.title")}
        description={t("announcements.delete.body")}
        confirmLabel={t("announcements.delete.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </Section>
  );
}
