import { create } from "@bufbuild/protobuf";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { live } from "@tokuly/api";
import { BarChart3, BellRing, MessageSquare, Plus, RefreshCw, Target, Trash2 } from "lucide-react";
import { type ComponentType, type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Page } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { PaneSpinner } from "#/components/ui/spinner";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { liveKeys, overlayClient, useGiftTiers } from "../api";
import {
  AlertWidget,
  ChatBoxWidget,
  GoalWidget,
  PollWidget,
  WIDGET_BASE_CSS,
} from "../overlay/widgets";
import { LiveChannelPicker, NoChannel, SecretField, Section, useLiveChannel } from "./shared";

const { OverlayKind, AlertKind, GoalKind, ChatMessageKind, ChatRole } = live;

const KINDS: {
  value: live.OverlayKind;
  key: string;
  icon: ComponentType<{ className?: string }>;
}[] = [
  { value: OverlayKind.CHAT_BOX, key: "chatBox", icon: MessageSquare },
  { value: OverlayKind.ALERT_BOX, key: "alertBox", icon: BellRing },
  { value: OverlayKind.GOAL, key: "goal", icon: Target },
  { value: OverlayKind.POLL, key: "poll", icon: BarChart3 },
];

const kindOf = (kind: live.OverlayKind) => KINDS.find((entry) => entry.value === kind) ?? KINDS[0]!;
const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "live" }));

/** The settings a widget of a kind starts from when it has none stored. */
function settingsOf(overlay: live.Overlay): live.OverlaySettings {
  if (overlay.settings?.widget.case) return overlay.settings;
  const widget =
    overlay.kind === OverlayKind.ALERT_BOX
      ? ({ case: "alertBox", value: create(live.AlertBoxSettingsSchema, {}) } as const)
      : overlay.kind === OverlayKind.GOAL
        ? ({
            case: "goal",
            value: create(live.GoalSettingsSchema, { kind: GoalKind.COINS, target: 1000n }),
          } as const)
        : overlay.kind === OverlayKind.POLL
          ? ({ case: "poll", value: create(live.PollWidgetSettingsSchema, {}) } as const)
          : ({ case: "chatBox", value: create(live.ChatBoxSettingsSchema, {}) } as const);
  return create(live.OverlaySettingsSchema, { widget });
}

/**
 * Draws its children inside a frame of their own, so a widget's CSS (which
 * styles `html` and `body` and sizes itself by the viewport) behaves as it
 * will in OBS and cannot leak into the app.
 */
function PreviewFrame({ css, children }: { css: string; children: ReactNode }) {
  const { t } = useTranslation("live");
  const frame = useRef<HTMLIFrameElement>(null);
  const [body, setBody] = useState<HTMLElement | null>(null);
  useEffect(() => {
    const element = frame.current;
    if (!element) return;
    // Only the document made from `srcDoc` counts: the frame starts out on a throwaway blank one.
    const attach = () => {
      const document = element.contentDocument;
      if (document?.URL === "about:srcdoc" && document.readyState === "complete")
        setBody(document.body);
    };
    attach();
    element.addEventListener("load", attach);
    return () => element.removeEventListener("load", attach);
  }, []);
  return (
    <div
      className="overflow-hidden rounded-2xl border-2 border-ink"
      // A checkerboard stands for whatever the broadcast shows behind the widget.
      style={{
        background: "repeating-conic-gradient(#5b5670 0% 25%, #3d3852 0% 50%) 50% / 24px 24px",
      }}
    >
      <iframe
        ref={frame}
        title={t("overlays.preview")}
        // Same colour scheme inside and out: otherwise the browser paints the frame opaque.
        srcDoc="<!doctype html><html style='color-scheme: light'><head><meta charset='utf-8'></head><body></body></html>"
        className="block h-72 w-full [color-scheme:light]"
        data-testid="overlay-preview"
      />
      {body &&
        createPortal(
          <>
            <style>{`${WIDGET_BASE_CSS}\n${css}`}</style>
            {children}
          </>,
          body,
        )}
    </div>
  );
}

/** What a widget shows in the preview: made-up chat, a sample alert, a goal half reached, a poll under way. */
function SampleWidget({
  overlay,
  settings,
}: {
  overlay: live.Overlay;
  settings: live.OverlaySettings;
}) {
  const { t } = useTranslation("live");
  const tiers = useGiftTiers().data?.tiers ?? [];
  const widget = settings.widget;
  if (widget.case === "alertBox")
    return (
      <AlertWidget
        settings={widget.value}
        alert={create(live.AlertSchema, {
          kind: AlertKind.GIFT,
          name: t("overlays.sample.name"),
          coins: 500n,
          tier: 5,
          message: t("overlays.sample.giftMessage"),
          test: true,
        })}
      />
    );
  if (widget.case === "goal")
    return (
      <GoalWidget
        settings={widget.value}
        goal={create(live.GoalProgressSchema, {
          overlayId: overlay.id,
          kind: widget.value.kind,
          title: widget.value.title,
          current: widget.value.target / 2n,
          target: widget.value.target,
        })}
      />
    );
  if (widget.case === "poll")
    return (
      <PollWidget
        settings={widget.value}
        poll={create(live.PollSchema, {
          id: "sample",
          question: t("overlays.sample.question"),
          options: [
            { text: t("overlays.sample.optionA"), voteCount: 12n },
            { text: t("overlays.sample.optionB"), voteCount: 7n },
          ],
          totalVotes: 19n,
        })}
      />
    );
  const line = (id: string, name: string, text: string, role = ChatRole.VIEWER) =>
    create(live.ChatMessageSchema, {
      id,
      kind: ChatMessageKind.TEXT,
      role,
      text,
      author: { user: { id, displayName: name } },
      offsetMs: BigInt(id.length),
    });
  return (
    <ChatBoxWidget
      tiers={tiers}
      settings={widget.case === "chatBox" ? widget.value : undefined}
      messages={[
        line("a", t("overlays.sample.name"), t("overlays.sample.chatA")),
        line("bb", t("overlays.sample.moderator"), t("overlays.sample.chatB"), ChatRole.MODERATOR),
        create(live.ChatMessageSchema, {
          id: "ccc",
          kind: ChatMessageKind.GIFT,
          text: t("overlays.sample.giftMessage"),
          author: { user: { id: "ccc", displayName: t("overlays.sample.name") } },
          gift: { coins: 500n, tier: 5 },
          offsetMs: 3n,
        }),
      ]}
    />
  );
}

function numberField(text: string, min: number, max: number): number {
  const value = Number.parseInt(text, 10);
  return Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : 0;
}

/** The options of a widget, by its kind. */
function SettingsFields({
  settings,
  onChange,
}: {
  settings: live.OverlaySettings;
  onChange(next: live.OverlaySettings): void;
}) {
  const { t } = useTranslation("live");
  const widget = settings.widget;
  const put = (next: live.OverlaySettings["widget"]) => onChange({ ...settings, widget: next });
  if (widget.case === "chatBox") {
    const value = widget.value;
    const set = (patch: Partial<live.ChatBoxSettings>) =>
      put({ case: "chatBox", value: { ...value, ...patch } });
    return (
      <>
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            label={t("overlays.chatBox.maxMessages")}
            type="number"
            min={1}
            max={50}
            value={String(value.maxMessages || 20)}
            onChange={(event) => set({ maxMessages: numberField(event.target.value, 1, 50) })}
          />
          <TextField
            label={t("overlays.chatBox.fadeSeconds")}
            hint={t("overlays.chatBox.fadeHint")}
            type="number"
            min={0}
            max={600}
            value={String(value.fadeSeconds)}
            onChange={(event) => set({ fadeSeconds: numberField(event.target.value, 0, 600) })}
          />
        </div>
        <div className="-mx-3 flex flex-col">
          <SwitchRow
            label={t("overlays.chatBox.avatars")}
            checked={!value.hideAvatars}
            onChange={(shown) => set({ hideAvatars: !shown })}
          />
          <SwitchRow
            label={t("overlays.chatBox.badges")}
            checked={!value.hideBadges}
            onChange={(shown) => set({ hideBadges: !shown })}
          />
          <SwitchRow
            label={t("overlays.chatBox.stamps")}
            checked={!value.hideStamps}
            onChange={(shown) => set({ hideStamps: !shown })}
          />
          <SwitchRow
            label={t("overlays.chatBox.gifts")}
            checked={!value.hideGifts}
            onChange={(shown) => set({ hideGifts: !shown })}
          />
        </div>
      </>
    );
  }
  if (widget.case === "alertBox") {
    const value = widget.value;
    const set = (patch: Partial<live.AlertBoxSettings>) =>
      put({ case: "alertBox", value: { ...value, ...patch } });
    return (
      <>
        <div className="-mx-3 flex flex-col">
          <SwitchRow
            label={t("overlays.alertBox.gifts")}
            checked={!value.hideGifts}
            onChange={(shown) => set({ hideGifts: !shown })}
          />
          <SwitchRow
            label={t("overlays.alertBox.subscribers")}
            checked={!value.hideSubscribers}
            onChange={(shown) => set({ hideSubscribers: !shown })}
          />
          <SwitchRow
            label={t("overlays.alertBox.raids")}
            checked={!value.hideRaids}
            onChange={(shown) => set({ hideRaids: !shown })}
          />
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            label={t("overlays.alertBox.minCoins")}
            type="number"
            min={0}
            value={String(value.minGiftCoins)}
            onChange={(event) =>
              set({ minGiftCoins: BigInt(numberField(event.target.value, 0, 100_000)) })
            }
          />
          <TextField
            label={t("overlays.alertBox.duration")}
            type="number"
            min={2}
            max={30}
            value={String(value.durationSeconds || 6)}
            onChange={(event) => set({ durationSeconds: numberField(event.target.value, 2, 30) })}
          />
        </div>
        <TextField
          label={t("overlays.alertBox.giftTemplate")}
          hint={t("overlays.alertBox.templateHint")}
          maxLength={200}
          value={value.giftTemplate}
          onChange={(event) => set({ giftTemplate: event.target.value })}
        />
        <TextField
          label={t("overlays.alertBox.subscriberTemplate")}
          maxLength={200}
          value={value.subscriberTemplate}
          onChange={(event) => set({ subscriberTemplate: event.target.value })}
        />
        <TextField
          label={t("overlays.alertBox.raidTemplate")}
          maxLength={200}
          value={value.raidTemplate}
          onChange={(event) => set({ raidTemplate: event.target.value })}
        />
      </>
    );
  }
  if (widget.case === "goal") {
    const value = widget.value;
    const set = (patch: Partial<live.GoalSettings>) =>
      put({ case: "goal", value: { ...value, ...patch } });
    return (
      <>
        <SelectField
          label={t("overlays.goal.kind")}
          value={String(value.kind || GoalKind.COINS)}
          onChange={(next) => set({ kind: Number(next) as live.GoalKind })}
          options={[
            { value: String(GoalKind.COINS), label: t("overlays.goal.coins") },
            { value: String(GoalKind.SUBSCRIBERS), label: t("overlays.goal.subscribers") },
          ]}
        />
        <div className="grid gap-3 sm:grid-cols-2">
          <TextField
            label={t("overlays.goal.title")}
            maxLength={60}
            value={value.title}
            onChange={(event) => set({ title: event.target.value })}
          />
          <TextField
            label={t("overlays.goal.target")}
            type="number"
            min={1}
            value={String(value.target)}
            onChange={(event) =>
              set({ target: BigInt(numberField(event.target.value, 1, 1_000_000_000)) })
            }
          />
        </div>
        <div className="-mx-3">
          <SwitchRow
            label={t("overlays.goal.numbers")}
            checked={!value.hideNumbers}
            onChange={(shown) => set({ hideNumbers: !shown })}
          />
        </div>
      </>
    );
  }
  if (widget.case === "poll") {
    const value = widget.value;
    const set = (patch: Partial<live.PollWidgetSettings>) =>
      put({ case: "poll", value: { ...value, ...patch } });
    return (
      <div className="-mx-3 flex flex-col">
        <SwitchRow
          label={t("overlays.poll.hideWhenIdle")}
          hint={t("overlays.poll.hideWhenIdleHint")}
          checked={value.hideWhenIdle}
          onChange={(hideWhenIdle) => set({ hideWhenIdle })}
        />
        <SwitchRow
          label={t("overlays.poll.voteCounts")}
          checked={!value.hideVoteCounts}
          onChange={(shown) => set({ hideVoteCounts: !shown })}
        />
      </div>
    );
  }
  return null;
}

/** One widget's configurator: its style and options with a preview that follows, and the URL for OBS. */
function OverlayEditor({
  overlay,
  presets,
  channelId,
}: {
  overlay: live.Overlay;
  presets: live.OverlayPreset[];
  channelId: string;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const [name, setName] = useState(overlay.name);
  const [preset, setPreset] = useState(overlay.preset);
  const [customCss, setCustomCss] = useState(overlay.customCss);
  const [settings, setSettings] = useState(() => settingsOf(overlay));
  const [dirty, setDirty] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [rotating, setRotating] = useState(false);
  const edit =
    <T,>(setter: (value: T) => void) =>
    (value: T) => {
      setter(value);
      setDirty(true);
    };
  const kindPresets = presets.filter((candidate) => candidate.kind === overlay.kind);
  const presetCss = kindPresets.find((candidate) => candidate.id === preset)?.css ?? "";

  const store = (next: live.Overlay | undefined) => {
    if (!next) return;
    queryClient.setQueryData<live.Overlay[]>(liveKeys.overlays(channelId), (current) =>
      current?.map((existing) => (existing.id === next.id ? next : existing)),
    );
  };
  const save = useMutation({
    mutationFn: (resetGoal: boolean) =>
      overlayClient().updateOverlay({
        overlayId: overlay.id,
        name: name.trim(),
        preset,
        customCss,
        settings,
        resetGoal,
      }),
    onSuccess: (response, resetGoal) => {
      store(response.overlay);
      setDirty(false);
      toast.success(t(resetGoal ? "overlays.goalReset" : "overlays.saved"));
    },
    onError: fail,
  });
  const rotate = useMutation({
    mutationFn: () => overlayClient().rotateOverlayToken({ overlayId: overlay.id }),
    onSuccess: (response) => {
      store(response.overlay);
      setRotating(false);
      toast.success(t("overlays.rotated"));
    },
    onError: fail,
  });
  const remove = useMutation({
    mutationFn: () => overlayClient().deleteOverlay({ overlayId: overlay.id }),
    onSuccess: () => {
      queryClient.setQueryData<live.Overlay[]>(liveKeys.overlays(channelId), (current) =>
        current?.filter((existing) => existing.id !== overlay.id),
      );
      toast.success(t("overlays.deleted"));
    },
    onError: fail,
  });
  const test = useMutation({
    mutationFn: (kind: live.AlertKind) => overlayClient().sendTestAlert({ channelId, kind }),
    onSuccess: (response) =>
      toast.success(t("overlays.testSent", { count: response.overlayCount })),
    onError: fail,
  });

  const kind = kindOf(overlay.kind);
  return (
    <div className="flex flex-col gap-4" data-testid="overlay-editor">
      <SecretField
        label={t("overlays.url")}
        value={overlay.url}
        kind="url"
        hint={t("overlays.urlHint")}
      />
      <PreviewFrame css={`${presetCss}\n${customCss}`}>
        <SampleWidget overlay={overlay} settings={settings} />
      </PreviewFrame>
      <div className="grid gap-3 sm:grid-cols-2">
        <TextField
          label={t("overlays.name")}
          value={name}
          maxLength={50}
          onChange={(event) => edit(setName)(event.target.value)}
        />
        <SelectField
          label={t("overlays.preset")}
          value={preset}
          onChange={edit(setPreset)}
          options={kindPresets.map((candidate) => ({
            value: candidate.id,
            label:
              (i18n.language.startsWith("ja") ? candidate.name?.ja : candidate.name?.en) ||
              candidate.id,
          }))}
        />
      </div>
      <SettingsFields settings={settings} onChange={edit(setSettings)} />
      <TextArea
        label={t("overlays.customCss")}
        hint={t("overlays.customCssHint")}
        value={customCss}
        maxLength={50_000}
        spellCheck={false}
        onChange={(event) => edit(setCustomCss)(event.target.value)}
      />
      <div className="flex flex-wrap gap-2">
        <Button
          loading={save.isPending && !save.variables}
          disabled={!dirty || !name.trim()}
          onClick={() => save.mutate(false)}
        >
          {t("common:actions.save")}
        </Button>
        {overlay.kind === OverlayKind.GOAL && (
          <Button variant="secondary" onClick={() => save.mutate(true)}>
            {t("overlays.resetGoal")}
          </Button>
        )}
        {overlay.kind === OverlayKind.ALERT_BOX &&
          (
            [
              [AlertKind.GIFT, "gift"],
              [AlertKind.SUBSCRIBER, "subscriber"],
              [AlertKind.RAID, "raid"],
            ] as const
          ).map(([alertKind, key]) => (
            <Button
              key={key}
              variant="secondary"
              size="sm"
              className="h-11"
              disabled={test.isPending}
              onClick={() => test.mutate(alertKind)}
            >
              {t(`overlays.test.${key}`)}
            </Button>
          ))}
        <span className="flex-1" />
        <Button
          variant="ghost"
          icon={<RefreshCw className="size-4" />}
          onClick={() => setRotating(true)}
        >
          {t("overlays.rotate")}
        </Button>
        <Button
          variant="ghost"
          className="text-danger"
          icon={<Trash2 className="size-4" />}
          onClick={() => setDeleting(true)}
        >
          {t("common:actions.delete")}
        </Button>
      </div>
      <ConfirmDialog
        open={rotating}
        onOpenChange={setRotating}
        title={t("overlays.rotateTitle")}
        description={t("overlays.rotateBody")}
        confirmLabel={t("overlays.rotate")}
        danger
        loading={rotate.isPending}
        onConfirm={() => rotate.mutate()}
      />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("overlays.deleteTitle", { name: overlay.name || t(`overlays.kinds.${kind.key}`) })}
        description={t("overlays.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}

function Overlays({ channelId }: { channelId: string }) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const [openId, setOpenId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [kind, setKind] = useState<live.OverlayKind>(OverlayKind.CHAT_BOX);
  const [name, setName] = useState("");
  const overlays = useQuery({
    queryKey: liveKeys.overlays(channelId),
    queryFn: async () => (await overlayClient().listOverlays({ channelId })).overlays,
    // The answers hold the widgets' secret URLs: not kept once the page is left.
    gcTime: 0,
  });
  const presets = useQuery({
    queryKey: liveKeys.presets,
    queryFn: async () => (await overlayClient().listOverlayPresets({})).presets,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const add = useMutation({
    mutationFn: () =>
      overlayClient().createOverlay({
        channelId,
        kind,
        name: name.trim() || t(`overlays.kinds.${kindOf(kind).key}`),
        preset: "",
        customCss: "",
      }),
    onSuccess: ({ overlay }) => {
      if (overlay) {
        queryClient.setQueryData<live.Overlay[]>(liveKeys.overlays(channelId), (current) => [
          ...(current ?? []),
          overlay,
        ]);
        setOpenId(overlay.id);
      }
      setCreating(false);
      setName("");
      toast.success(t("overlays.created"));
    },
    onError: fail,
  });

  if (overlays.isPending || presets.isPending) return <PaneSpinner />;
  const list = overlays.data ?? [];
  return (
    <>
      <p className="mb-4 text-sm leading-relaxed text-ink-soft">{t("overlays.lead")}</p>
      {list.length === 0 ? (
        <EmptyState
          emoji="🧩"
          title={t("overlays.emptyTitle")}
          body={t("overlays.emptyBody")}
          action={
            <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
              {t("overlays.add")}
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-4">
          {list.map((overlay) => {
            const entry = kindOf(overlay.kind);
            const Icon = entry.icon;
            const open = openId === overlay.id;
            return (
              <Section
                key={overlay.id}
                title={overlay.name || t(`overlays.kinds.${entry.key}`)}
                lead={t(`overlays.kinds.${entry.key}`)}
                actions={
                  <Button
                    variant="secondary"
                    size="sm"
                    aria-expanded={open}
                    icon={<Icon className="size-4" />}
                    onClick={() => setOpenId(open ? null : overlay.id)}
                  >
                    {t(open ? "overlays.close" : "overlays.configure")}
                  </Button>
                }
                className={cn(!open && "[&>div:last-child]:hidden")}
              >
                {open && (
                  <OverlayEditor
                    key={overlay.token}
                    overlay={overlay}
                    presets={presets.data ?? []}
                    channelId={channelId}
                  />
                )}
              </Section>
            );
          })}
          {list.length < 20 && (
            <Button
              variant="secondary"
              className="self-start"
              icon={<Plus className="size-4" />}
              onClick={() => setCreating(true)}
            >
              {t("overlays.add")}
            </Button>
          )}
        </div>
      )}
      <Dialog
        open={creating}
        onOpenChange={setCreating}
        title={t("overlays.addTitle")}
        footer={
          <Button loading={add.isPending} onClick={() => add.mutate()}>
            {t("overlays.create")}
          </Button>
        }
      >
        <fieldset className="grid grid-cols-2 gap-2">
          <legend className="mb-2 px-1 text-sm font-bold text-ink-soft">
            {t("overlays.kind")}
          </legend>
          {KINDS.map((entry) => {
            const Icon = entry.icon;
            return (
              <button
                key={entry.key}
                type="button"
                aria-pressed={kind === entry.value}
                onClick={() => setKind(entry.value)}
                className={cn(
                  "flex flex-col items-start gap-1 rounded-2xl border-2 p-3 text-left",
                  kind === entry.value ? "border-ink bg-brand-soft" : "border-line",
                )}
              >
                <Icon className="size-5" />
                <span className="text-sm font-black">{t(`overlays.kinds.${entry.key}`)}</span>
                <span className="text-xs text-ink-soft">
                  {t(`overlays.kindHints.${entry.key}`)}
                </span>
              </button>
            );
          })}
        </fieldset>
        <TextField
          className="mt-4"
          label={t("overlays.name")}
          hint={t("overlays.nameHint")}
          value={name}
          maxLength={50}
          onChange={(event) => setName(event.target.value)}
        />
      </Dialog>
    </>
  );
}

/** `/studio/live/overlays`: the OBS widgets of a channel and their configurators. */
export function OverlaysScreen() {
  const { t } = useTranslation("live");
  const { query, channels, channel } = useLiveChannel();
  const channelId = channel?.summary?.id;
  return (
    <Page title={t("overlays.title")} back="/studio/live">
      {query.isPending ? (
        <PaneSpinner />
      ) : !channelId ? (
        <NoChannel />
      ) : (
        <>
          <LiveChannelPicker channels={channels} selectedId={channelId} />
          <Overlays key={channelId} channelId={channelId} />
        </>
      )}
    </Page>
  );
}
