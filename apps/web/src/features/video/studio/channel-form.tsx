import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, media, video } from "@tokuly/api";
import { Check, ImagePlus, Plus, Star, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Spinner } from "#/components/ui/spinner";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { uploadAndProcess } from "#/lib/upload";
import { useTranslation } from "#/lib/use-translation";
import { channelClient, useMyChannels, videoKeys } from "../api";
import { FormSection, selectStudioChannel } from "./shared";
import { useSetDefaultChannel } from "./studio-home";

const MAX_LINKS = 5;
const NO_TRAILER = "none";

interface ArtDraft {
  /** What the form shows: the stored image, or a preview of the file just chosen. */
  url: string;
  /** A newly uploaded asset; `""` asks to remove the stored image; undefined leaves it as it is. */
  assetId?: string;
  uploading: boolean;
}

/** `/studio/channels/new`: the first step of becoming a creator. */
export function NewChannelScreen() {
  const { t } = useTranslation("video");
  const channels = useMyChannels();
  const full = channels.data && channels.data.channels.length >= channels.data.maxChannels;
  return (
    <Page title={t("studio.channel.newTitle")} back="/studio">
      {full ? (
        <EmptyState
          emoji="🖐️"
          title={t("studio.channel.limit", { count: channels.data?.maxChannels ?? 5 })}
          body={t("studio.channel.limitBody")}
        />
      ) : (
        <ChannelForm />
      )}
    </Page>
  );
}

/** `/studio/channels/<id>`: everything about one of the person's channels. */
export function EditChannelScreen({ channelId }: { channelId: string }) {
  const { t } = useTranslation("video");
  const channels = useMyChannels();
  const channel = channels.data?.channels.find(
    (candidate) => candidate.summary?.id === channelId || candidate.summary?.handle === channelId,
  );
  return (
    <Page title={t("studio.channel.editTitle")} back="/studio">
      {channels.isPending ? (
        <Skeleton className="h-96 rounded-card" />
      ) : channel ? (
        <ChannelForm key={channel.summary?.id} channel={channel} />
      ) : (
        <EmptyState emoji="📡" title={t("channel.notFound")} />
      )}
    </Page>
  );
}

function ChannelForm({ channel }: { channel?: video.Channel }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const summary = channel?.summary;
  const [name, setName] = useState(summary?.name ?? "");
  const [handle, setHandle] = useState(summary?.handle ?? "");
  const [description, setDescription] = useState(channel?.description ?? "");
  const [links, setLinks] = useState(
    () => channel?.links.map((link) => ({ title: link.title, url: link.url })) ?? [],
  );
  const [icon, setIcon] = useState<ArtDraft>({ url: summary?.icon?.url ?? "", uploading: false });
  const [banner, setBanner] = useState<ArtDraft>({
    url: channel?.banner?.url ?? "",
    uploading: false,
  });
  const [trailer, setTrailer] = useState(channel?.trailerVideoId || NO_TRAILER);
  const [deleting, setDeleting] = useState(false);
  const setDefault = useSetDefaultChannel();

  // The handle is checked while it is typed, a moment after the last key.
  const [checked, setChecked] = useState(handle);
  useEffect(() => {
    const timer = setTimeout(() => setChecked(handle.trim()), 350);
    return () => clearTimeout(timer);
  }, [handle]);
  const handleChanged = handle.trim().toLowerCase().replace(/^@/, "") !== (summary?.handle ?? "");
  const availability = useQuery({
    queryKey: ["video", "handle-check", checked],
    queryFn: () => channelClient().checkChannelHandle({ handle: checked }),
    enabled: checked.length > 0 && handleChanged,
    staleTime: 10_000,
  });
  const settled =
    checked === handle.trim() && availability.data !== undefined && !availability.isFetching;
  const handleError =
    handleChanged && settled && !availability.data.available
      ? t(`studio.channel.handle.${availability.data.reason || "HANDLE_INVALID"}`)
      : undefined;
  const handleOk = handleChanged && settled && availability.data.available;

  const publicVideos = useQuery({
    queryKey: videoKeys.channelVideos(
      summary?.id ?? "",
      video.ChannelVideoSort.NEWEST,
      video.VideoOrigin.UNSPECIFIED,
    ),
    queryFn: () =>
      channelClient().listChannelVideos({ channelId: summary?.id ?? "", page: { limit: 50 } }),
    enabled: Boolean(summary),
  });

  const save = useMutation({
    mutationFn: async () => {
      const cleanLinks = links
        .map((link) => ({ title: link.title.trim(), url: link.url.trim() }))
        .filter((link) => link.url);
      if (channel && summary) {
        const response = await channelClient().updateChannel({
          channelId: summary.id,
          name: name.trim(),
          handle: handleChanged ? handle.trim() : undefined,
          description: description.trim(),
          iconAssetId: icon.assetId,
          bannerAssetId: banner.assetId,
          links: { links: cleanLinks },
          trailerVideoId: trailer === NO_TRAILER ? "" : trailer,
        });
        return response.channel;
      }
      const response = await channelClient().createChannel({
        name: name.trim(),
        handle: handle.trim(),
        description: description.trim(),
        iconAssetId: icon.assetId ?? "",
        bannerAssetId: banner.assetId ?? "",
        links: cleanLinks,
      });
      return response.channel;
    },
    onSuccess: (saved) => {
      void queryClient.invalidateQueries({ queryKey: videoKeys.channels });
      if (channel) {
        toast.success(t("studio.channel.saved"));
      } else {
        toast.success(t("studio.channel.created"));
        if (saved?.summary) selectStudioChannel(saved.summary.id);
      }
      void navigate({ to: "/studio" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });

  const busy = icon.uploading || banner.uploading;
  const canSave =
    name.trim().length > 0 &&
    handle.trim().length >= 3 &&
    !handleError &&
    !busy &&
    (!handleChanged || handleOk);

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSave) save.mutate();
      }}
    >
      {!channel && (
        <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("studio.channel.newLead")}</p>
      )}

      <FormSection title={t("studio.channel.basics")}>
        <TextField
          label={t("studio.channel.name")}
          value={name}
          maxLength={50}
          placeholder={t("studio.channel.namePlaceholder")}
          onChange={(event) => setName(event.target.value)}
        />
        <TextField
          label={t("studio.channel.handleLabel")}
          value={handle}
          maxLength={25}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          placeholder="yuru_games"
          hint={handleOk ? t("studio.channel.handle.available") : t("studio.channel.handle.hint")}
          error={handleError}
          trailing={
            handleChanged && handle.trim() ? (
              settled ? (
                handleOk && <Check className="mr-2 size-5 text-success" aria-hidden />
              ) : (
                <Spinner className="mr-2 size-4 text-ink-faint" />
              )
            ) : undefined
          }
          onChange={(event) => setHandle(event.target.value.replace(/\s/g, ""))}
        />
        <TextArea
          label={t("studio.channel.description")}
          value={description}
          maxLength={1000}
          placeholder={t("studio.channel.descriptionPlaceholder")}
          onChange={(event) => setDescription(event.target.value)}
        />
      </FormSection>

      <FormSection title={t("studio.channel.art")} lead={t("studio.channel.artLead")}>
        <div className="flex flex-col gap-4 sm:flex-row">
          <ArtField
            label={t("studio.channel.icon")}
            shape="icon"
            purpose={media.AssetPurpose.CHANNEL_ICON}
            draft={icon}
            onChange={setIcon}
          />
          <ArtField
            label={t("studio.channel.banner")}
            shape="banner"
            purpose={media.AssetPurpose.CHANNEL_BANNER}
            draft={banner}
            onChange={setBanner}
          />
        </div>
      </FormSection>

      <FormSection title={t("studio.channel.links")} lead={t("studio.channel.linksLead")}>
        {links.map((link, index) => (
          <div key={index} className="flex items-start gap-2">
            <div className="grid min-w-0 flex-1 gap-2 sm:grid-cols-[10rem_minmax(0,1fr)]">
              <TextField
                aria-label={t("studio.channel.linkTitle")}
                value={link.title}
                maxLength={30}
                placeholder={t("studio.channel.linkTitle")}
                onChange={(event) =>
                  setLinks(
                    links.map((entry, at) =>
                      at === index ? { ...entry, title: event.target.value } : entry,
                    ),
                  )
                }
              />
              <TextField
                aria-label={t("studio.channel.linkUrl")}
                value={link.url}
                maxLength={500}
                type="url"
                inputMode="url"
                placeholder="https://"
                onChange={(event) =>
                  setLinks(
                    links.map((entry, at) =>
                      at === index ? { ...entry, url: event.target.value } : entry,
                    ),
                  )
                }
              />
            </div>
            <Button
              variant="ghost"
              size="icon"
              aria-label={t("studio.channel.removeLink")}
              onClick={() => setLinks(links.filter((_, at) => at !== index))}
            >
              <X className="size-5" />
            </Button>
          </div>
        ))}
        {links.length < MAX_LINKS && (
          <Button
            variant="secondary"
            className="self-start"
            icon={<Plus className="size-4" />}
            onClick={() => setLinks([...links, { title: "", url: "" }])}
          >
            {t("studio.channel.addLink")}
          </Button>
        )}
      </FormSection>

      {channel && summary && (
        <FormSection title={t("studio.channel.featured")} lead={t("studio.channel.featuredLead")}>
          <SelectField
            label={t("studio.channel.trailer")}
            value={trailer}
            onChange={setTrailer}
            options={[
              { value: NO_TRAILER, label: t("studio.channel.noTrailer") },
              ...(publicVideos.data?.videos ?? [])
                .filter((entry) => entry.visibility === common.Visibility.PUBLIC)
                .map((entry) => ({ value: entry.shortId, label: entry.title })),
              // A trailer set earlier stays selectable even when it is not among the newest fifty.
              ...(trailer !== NO_TRAILER &&
              !publicVideos.data?.videos.some((entry) => entry.shortId === trailer)
                ? [{ value: trailer, label: trailer }]
                : []),
            ]}
          />
          {!channel.viewer?.isDefault && (
            <div className="flex flex-wrap items-center gap-3 rounded-2xl bg-surface-2 p-3">
              <p className="min-w-0 flex-1 text-sm leading-relaxed text-ink-soft">
                {t("studio.channel.defaultLead")}
              </p>
              <Button
                variant="secondary"
                icon={<Star className="size-4" />}
                loading={setDefault.isPending}
                onClick={() => setDefault.mutate(summary.id)}
              >
                {t("studio.channel.makeDefault")}
              </Button>
            </div>
          )}
        </FormSection>
      )}

      <div className="flex justify-end">
        <Button type="submit" size="lg" disabled={!canSave} loading={save.isPending}>
          {channel ? t("actions.save") : t("studio.channel.create")}
        </Button>
      </div>

      {channel && summary && (
        <section className="rounded-card border-2 border-dashed border-danger p-4">
          <h2 className="font-black text-danger">{t("studio.channel.dangerTitle")}</h2>
          <p className="mt-1 text-sm leading-relaxed text-ink-soft">
            {t("studio.channel.dangerLead")}
          </p>
          <Button
            variant="danger"
            className="mt-3"
            icon={<Trash2 className="size-4" />}
            onClick={() => setDeleting(true)}
          >
            {t("studio.channel.delete")}
          </Button>
          <DeleteChannelDialog
            open={deleting}
            onOpenChange={setDeleting}
            channelId={summary.id}
            handle={summary.handle}
            name={summary.name}
          />
        </section>
      )}
    </form>
  );
}

/** An image of the channel: choose a file, see it at once, take it away again. */
function ArtField({
  label,
  shape,
  purpose,
  draft,
  onChange,
}: {
  label: string;
  shape: "icon" | "banner";
  purpose: media.AssetPurpose;
  draft: ArtDraft;
  onChange(draft: ArtDraft): void;
}) {
  const { t } = useTranslation("video");
  const input = useRef<HTMLInputElement>(null);
  // The preview is a blob URL of this page: release it when it is replaced or the form goes away.
  const preview = draft.url.startsWith("blob:") ? draft.url : undefined;
  useEffect(() => () => void (preview && URL.revokeObjectURL(preview)), [preview]);

  const choose = async (file: File) => {
    const previous = draft;
    const url = URL.createObjectURL(file);
    onChange({ url, uploading: true });
    try {
      const asset = await uploadAndProcess(file, { purpose });
      onChange({ url, assetId: asset.id, uploading: false });
    } catch (error) {
      onChange(previous);
      toast.error(errorMessage(error, { namespace: "video" }));
    }
  };

  return (
    <div className={cn("flex flex-col gap-1.5", shape === "banner" && "min-w-0 flex-1")}>
      <span className="px-1 text-sm font-bold text-ink-soft">{label}</span>
      <div
        className={cn(
          "relative overflow-hidden border-2 border-line bg-surface-2",
          shape === "icon" ? "size-28 rounded-[1.6rem]" : "aspect-[4/1] w-full rounded-2xl",
        )}
      >
        {draft.url ? (
          <img src={draft.url} alt="" className="size-full object-cover" />
        ) : (
          <span className="grid size-full place-items-center text-ink-faint">
            <ImagePlus className="size-7" />
          </span>
        )}
        {draft.uploading && (
          <span className="absolute inset-0 grid place-items-center bg-overlay text-white">
            <Spinner className="size-6" />
          </span>
        )}
      </div>
      <div className="flex gap-2">
        <Button
          variant="secondary"
          size="sm"
          icon={<ImagePlus className="size-4" />}
          disabled={draft.uploading}
          onClick={() => input.current?.click()}
        >
          {draft.url ? t("studio.channel.changeImage") : t("studio.channel.chooseImage")}
        </Button>
        {draft.url && !draft.uploading && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onChange({ url: "", assetId: "", uploading: false })}
          >
            {t("studio.channel.removeImage")}
          </Button>
        )}
      </div>
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        className="hidden"
        aria-label={label}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) void choose(file);
        }}
      />
    </div>
  );
}

/** Deleting a channel takes its videos with it: the handle has to be typed to go through with it. */
function DeleteChannelDialog({
  open,
  onOpenChange,
  channelId,
  handle,
  name,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  channelId: string;
  handle: string;
  name: string;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [typed, setTyped] = useState("");
  const remove = useMutation({
    mutationFn: () => channelClient().deleteChannel({ channelId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: videoKeys.all });
      toast.success(t("studio.channel.deleted"));
      onOpenChange(false);
      void navigate({ to: "/studio" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });
  const matches = typed.trim().replace(/^@/, "").toLowerCase() === handle;
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("studio.channel.confirmDelete", { name })}
      description={t("studio.channel.confirmDeleteBody")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("actions.cancel")}
          </Button>
          <Button
            variant="danger"
            disabled={!matches}
            loading={remove.isPending}
            onClick={() => remove.mutate()}
          >
            {t("studio.channel.delete")}
          </Button>
        </>
      }
    >
      <TextField
        label={t("studio.channel.typeHandle", { handle })}
        value={typed}
        autoCapitalize="none"
        spellCheck={false}
        placeholder={handle}
        onChange={(event) => setTyped(event.target.value)}
      />
    </Dialog>
  );
}
