import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorReason, type feed } from "@tokuly/api";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { useTranslation } from "#/lib/use-translation";
import { newsAdminClient, newsKeys } from "../api";
import { MAX_TAGS, parseTags } from "./input";
import { staffErrorMessage } from "./parts";

/** A field's value when it differs from what the source has, else `undefined` (leave it alone). */
const changed = (value: string, before: string): string | undefined =>
  value.trim() === before ? undefined : value.trim();

/** Reasons the API gives for one field of the form. */
const FIELD_OF_REASON: Record<string, "feedUrl" | "name" | "siteUrl" | "language" | "tags"> = {
  INVALID_FEED_URL: "feedUrl",
  FEED_URL_NOT_ALLOWED: "feedUrl",
  NEWS_SOURCE_EXISTS: "feedUrl",
  INVALID_NAME: "name",
  INVALID_SITE_URL: "siteUrl",
  INVALID_LANGUAGE: "language",
  INVALID_TAG: "tags",
  TOO_MANY_TAGS: "tags",
};

/**
 * Adds an RSS/Atom feed as a news source, or changes one. Name and site URL
 * may stay empty: the feed says what it is called on the first fetch.
 */
export function SourceDialog({
  source,
  onClose,
  onSaved,
}: {
  /** The source to change; absent to add one. */ source?: feed.ManagedNewsSource;
  onClose(): void /** The source was added or changed (a fetch may have been queued). */;
  onSaved?(): void;
}) {
  const { t } = useTranslation(["news", "common"]);
  const queryClient = useQueryClient();
  const [feedUrl, setFeedUrl] = useState(source?.feedUrl ?? "");
  const [name, setName] = useState(source?.name ?? "");
  const [siteUrl, setSiteUrl] = useState(source?.siteUrl ?? "");
  const [language, setLanguage] = useState(source?.language ?? "ja");
  const [tagsText, setTagsText] = useState(source?.defaultTags.join(", ") ?? "");
  const [enabled, setEnabled] = useState(source?.enabled ?? true);
  const [failed, setFailed] = useState<{ field?: string; message: string }>();
  const tags = parseTags(tagsText);

  const save = useMutation({
    mutationFn: async (): Promise<void> => {
      if (source) {
        // Only what was changed is sent: a new feed URL, for one, makes the source start over.
        await newsAdminClient().updateNewsSource({
          sourceId: source.id,
          feedUrl: changed(feedUrl, source.feedUrl),
          name: changed(name, source.name),
          siteUrl: changed(siteUrl, source.siteUrl),
          language: changed(language, source.language),
          defaultTags: tags.join("\n") === source.defaultTags.join("\n") ? undefined : { tags },
          enabled: enabled === source.enabled ? undefined : enabled,
        });
      } else {
        await newsAdminClient().createNewsSource({
          feedUrl: feedUrl.trim(),
          name: name.trim(),
          siteUrl: siteUrl.trim(),
          language: language.trim(),
          defaultTags: tags,
          disabled: !enabled,
        });
      }
    },
    onSuccess: () => {
      toast.success(t(source ? "news:staff.sources.saved" : "news:staff.sources.created"));
      onSaved?.();
      onClose();
      return Promise.all([
        queryClient.invalidateQueries({ queryKey: newsKeys.admin.sources }),
        queryClient.invalidateQueries({ queryKey: newsKeys.sources }),
      ]);
    },
    onError: (error) =>
      setFailed({
        field: FIELD_OF_REASON[errorReason(error) ?? ""],
        message: staffErrorMessage(error),
      }),
  });
  const errorFor = (field: string) => (failed?.field === field ? failed.message : undefined);
  const edit = (set: (value: string) => void) => (event: { target: { value: string } }) => {
    set(event.target.value);
    setFailed(undefined);
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t(source ? "news:staff.sources.editTitle" : "news:staff.sources.addTitle")}
      description={t("news:staff.sources.dialogLead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button
            onClick={() => save.mutate()}
            disabled={!feedUrl.trim() || tags.length > MAX_TAGS}
            loading={save.isPending}
          >
            {t(source ? "common:actions.save" : "news:staff.sources.add")}
          </Button>
        </>
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          if (feedUrl.trim() && tags.length <= MAX_TAGS) save.mutate();
        }}
      >
        <TextField
          label={t("news:staff.sources.feedUrl")}
          hint={t("news:staff.sources.feedUrlHint")}
          error={errorFor("feedUrl")}
          type="url"
          inputMode="url"
          value={feedUrl}
          onChange={edit(setFeedUrl)}
          placeholder="https://example.com/rss.xml"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
          required
        />
        <TextField
          label={t("news:staff.sources.name")}
          hint={t("news:staff.sources.nameHint")}
          error={errorFor("name")}
          value={name}
          onChange={edit(setName)}
          autoComplete="off"
        />
        <TextField
          label={t("news:staff.sources.siteUrl")}
          hint={t("news:staff.sources.siteUrlHint")}
          error={errorFor("siteUrl")}
          type="url"
          inputMode="url"
          value={siteUrl}
          onChange={edit(setSiteUrl)}
          placeholder="https://example.com"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        <TextField
          label={t("news:staff.sources.language")}
          hint={t("news:staff.sources.languageHint")}
          error={errorFor("language")}
          value={language}
          onChange={edit(setLanguage)}
          placeholder="ja"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        <TextField
          label={t("news:staff.sources.tags")}
          hint={t("news:staff.sources.tagsHint", { count: tags.length })}
          error={
            errorFor("tags") ??
            (tags.length > MAX_TAGS ? t("news:staff.errors.TOO_MANY_TAGS") : undefined)
          }
          value={tagsText}
          onChange={edit(setTagsText)}
          placeholder="tech, ゲーム"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        <SwitchRow
          label={t("news:staff.sources.enabled")}
          hint={t("news:staff.sources.enabledHint")}
          checked={enabled}
          onChange={setEnabled}
        />
        {failed && !failed.field && (
          <p role="alert" className="px-1 text-sm font-normal text-danger">
            {failed.message}
          </p>
        )}
        {/* Enter in a field saves, like the button. */}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
