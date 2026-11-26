import { useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import {
  type Audience,
  applyCompletion,
  audienceFields,
  audienceOption,
  charCount,
  completionAt,
  NOTE_MAX_CHARS,
  newPoll,
  noteAudience,
  type PollDraft,
  pollInput,
  pollReady,
  replyAudience,
} from "@tokuly/core/notes";
import { randomUUID } from "expo-crypto";
import { Image } from "expo-image";
import { router, useNavigation, usePreventRemove } from "expo-router";
import { BarChart3, ImagePlus, TriangleAlert, X } from "lucide-react-native";
import { type ComponentRef, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  KeyboardAvoidingView,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { useTheme } from "#/lib/theme";
import { agreeToRules, type NoteContent, postNote, saveDraft } from "./actions";
import { AudienceSheet } from "./audience-sheet";
import type { ComposeParams } from "./compose-href";
import {
  draftClient,
  noteClient,
  useDefaultAudience,
  useDraft,
  useHashtagSearch,
  useNote,
  useNoteRules,
} from "./data";
import { imageSource } from "./media";
import { type NotePhoto, useNotePhotos } from "./photos";
import { PollEditor } from "./poll-editor";
import { NoteRichText } from "./rich-text";

/** The count shows once a note gets long. */
const COUNT_FROM = NOTE_MAX_CHARS - 500;

/**
 * Writing a note: a new one, a reply or a quote, or going on with a draft.
 * The rules come first if the account has not agreed to them yet.
 */
export function ComposeScreen(params: ComposeParams) {
  const { t } = useTranslation("notes");
  const rules = useNoteRules();
  const draft = useDraft(params.draft);
  // A draft of a reply or a quote brings its note along: the reply rule and the card need it.
  const replyToId = params.replyTo ?? (draft.data?.content?.replyToId || undefined);
  const quoteId = params.quote ?? (draft.data?.content?.quoteOfId || undefined);
  const replyTo = useNote(replyToId);
  const quote = useNote(quoteId);
  const loading =
    rules.isPending ||
    (params.draft && draft.isPending) ||
    (replyToId && replyTo.isPending) ||
    (quoteId && quote.isPending);
  if (loading) return <Spinner />;
  // The note to answer or quote, or the draft, is gone or hidden: nothing to write against.
  // (A draft whose note went away still opens: the server says so when it is posted.)
  if (
    (params.replyTo && !replyTo.data) ||
    (params.quote && !quote.data) ||
    (params.draft && !draft.data)
  )
    return <EmptyState emoji="🫥" title={t("thread.notFound")} body={t("thread.notFoundBody")} />;
  if (rules.data && !rules.data.agreed) return <RulesAgreement rules={rules.data} />;
  return (
    <Composer
      replyTo={replyTo.data ?? undefined}
      quote={quote.data ?? undefined}
      draft={draft.data ?? undefined}
      text={params.text}
    />
  );
}

function RulesAgreement({ rules }: { rules: notes.GetNoteRulesResponse }) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="note-rules">
      <ScrollView contentContainerClassName="gap-3 px-4 pt-4 pb-6">
        <Text className="text-lg font-black text-ink">{t("rules.title")}</Text>
        <Text className="text-ink-soft">
          {rules.agreedVersion > 0 ? t("rules.updated") : t("rules.lead")}
        </Text>
        {rules.rules.map((rule, index) => (
          <View key={index} className="gap-1 rounded-card bg-surface p-4">
            <Text className="font-black text-ink">{rule.title}</Text>
            <Text className="text-sm text-ink-soft">{rule.body}</Text>
          </View>
        ))}
      </ScrollView>
      <View className="flex-row gap-3 px-4 pb-3">
        <Button variant="ghost" onPress={() => router.back()}>
          {t("rules.later")}
        </Button>
        <View className="flex-1">
          <Button
            testID="note-rules-agree"
            block
            loading={busy}
            onPress={async () => {
              setBusy(true);
              try {
                await agreeToRules(queryClient, rules.version);
              } catch (error) {
                Alert.alert(errorMessage(error, { namespace: "notes" }));
              } finally {
                setBusy(false);
              }
            }}
          >
            {t("rules.agree")}
          </Button>
        </View>
      </View>
    </SafeAreaView>
  );
}

const photosOfDraft = (draft: notes.Draft | undefined): NotePhoto[] =>
  (draft?.attachments ?? []).map((attachment) => ({
    key: attachment.assetId,
    uri: imageSource(attachment.image, 120) ?? attachment.url,
    assetId: attachment.assetId,
    progress: 1,
  }));

const audienceOfDraft = (content: notes.DraftContent | undefined): Audience | undefined =>
  content && content.visibility !== 0
    ? {
        visibility: content.visibility,
        circleId: content.circleId || undefined,
        directUserIds: content.directUserIds.length > 0 ? content.directUserIds : undefined,
      }
    : undefined;

function Composer({
  replyTo,
  quote,
  draft,
  text: startText,
}: {
  replyTo?: notes.Note;
  quote?: notes.Note;
  draft?: notes.Draft;
  text?: string;
}) {
  const { t } = useTranslation("notes");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const navigation = useNavigation();
  const content = draft?.content;
  const [text, setText] = useState(content?.text ?? startText ?? "");
  const [caret, setCaret] = useState(text.length);
  const [chosen, setChosen] = useState<Audience | undefined>(audienceOfDraft(content));
  const [picking, setPicking] = useState(false);
  const [warning, setWarning] = useState(content?.contentWarning ?? false);
  const [warningText, setWarningText] = useState(content?.contentWarningText ?? "");
  const [poll, setPoll] = useState<PollDraft | undefined>(() =>
    content?.poll
      ? {
          choices: content.poll.choices,
          multiple: content.poll.multiple,
          durationSeconds: Number(content.poll.durationSeconds),
          results: content.poll.resultsVisibility,
        }
      : undefined,
  );
  const photos = useNotePhotos(photosOfDraft(draft));
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState<"post" | "draft">();
  const [idempotencyKey] = useState(() => randomUUID());

  const completion = completionAt(text, caret);
  const hashtag =
    completion?.kind === "hashtag" && completion.query.length > 0 ? completion : undefined;
  const suggestions = useHashtagSearch(hashtag?.query ?? "", Boolean(hashtag));

  // What the note is shown as: the choice (or the account's default), never wider than the note answered.
  const answered = replyTo ? noteAudience(replyTo) : undefined;
  const ceiling = answered ? audienceOption(answered) : undefined;
  // Nothing chosen: the account's usual audience, which the server applies (and narrows for a
  // reply) by itself; it is read here only so the button says who that is.
  const usual = useDefaultAudience().data;
  const wanted = chosen ?? usual;
  const shownAudience = wanted ? replyAudience(wanted, answered) : undefined;

  const length = charCount(text.trim());
  const over = length > NOTE_MAX_CHARS;
  const hasContent = text.trim() !== "" || photos.photos.length > 0 || poll !== undefined;
  const canPost =
    hasContent && !over && !photos.busy && !photos.failed && !busy && (!poll || pollReady(poll));
  // Set once the words are posted or kept: the screen goes without asking. Kept out of
  // `unsaved` too, or the guard comes back when `busy` clears while the screen is leaving.
  const done = useRef(false);
  const unsaved = touched && hasContent && !busy && !done.current;

  const draftContent = (): NoteContent => ({
    text: text.trim(),
    assetIds: photos.assetIds,
    ...audienceFields(chosen),
    replyToId: replyTo?.id ?? content?.replyToId ?? "",
    quoteOfId: quote?.id ?? content?.quoteOfId ?? "",
    contentWarning: warning,
    contentWarningText: warning ? warningText : "",
    poll: poll ? pollInput(poll) : undefined,
    channelId: content?.channelId ?? "",
  });

  const leave = () => {
    done.current = true;
    router.back();
  };

  const keepAsDraft = async () => {
    const client = draftClient();
    setBusy("draft");
    try {
      // A draft waiting for its time keeps it: only the words change here.
      await saveDraft(queryClient, client, draft?.id ?? "", draftContent(), draft?.scheduledAt);
      Alert.alert(t("toast.draftSaved"));
      leave();
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "notes" }));
    } finally {
      setBusy(undefined);
    }
  };

  const post = async () => {
    const client = noteClient();
    const drafts = draftClient();
    setBusy("post");
    try {
      await postNote(queryClient, client, { ...draftContent(), idempotencyKey });
      if (draft) await drafts.deleteDraft({ draftId: draft.id }).catch(() => {});
      leave();
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "notes" }));
    } finally {
      setBusy(undefined);
    }
  };

  // Leaving with words not posted: keep them as a draft, throw them away, or go on writing.
  usePreventRemove(unsaved, ({ data }) => {
    if (done.current) return navigation.dispatch(data.action);
    Alert.alert(tm("notes.leaveTitle"), undefined, [
      { text: tm("notes.keepWriting"), style: "cancel" },
      {
        text: t("compose.discardDraft"),
        style: "destructive",
        onPress: () => navigation.dispatch(data.action),
      },
      { text: t("compose.saveDraft"), onPress: () => void keepAsDraft() },
    ]);
  });

  const edit = (next: string) => {
    setText(next);
    setTouched(true);
  };
  const title = replyTo
    ? t("compose.title.reply")
    : quote
      ? t("compose.title.quote")
      : t("compose.title.new");
  useEffect(() => navigation.setOptions({ title }), [navigation, title]);
  // Android draws edge to edge: the window is not resized for the keyboard, so the toolbar is
  // lifted above it. The view starts below the header, which the keyboard's position counts.
  const screen = useRef<ComponentRef<typeof View>>(null);
  const [top, setTop] = useState(0);
  return (
    <SafeAreaView
      ref={screen}
      edges={["bottom"]}
      className="flex-1 bg-bg"
      testID="note-composer"
      onLayout={() => screen.current?.measureInWindow((_x, y) => setTop(y))}
    >
      <KeyboardAvoidingView className="flex-1" behavior="padding" keyboardVerticalOffset={top}>
        <ScrollView
          contentContainerClassName="gap-3 px-4 pt-4 pb-6"
          keyboardShouldPersistTaps="handled"
        >
          {replyTo && (
            <View className="gap-1 rounded-2xl bg-surface p-3">
              <Text className="text-xs font-bold text-ink-soft">
                {t("compose.replyingTo", { handle: replyTo.author?.handle ?? "" })}
              </Text>
              <NoteRichText note={replyTo} numberOfLines={3} />
            </View>
          )}
          <Pressable
            testID="note-audience"
            accessibilityRole="button"
            onPress={() => setPicking(true)}
            className="self-start rounded-full bg-surface px-3 py-1.5"
          >
            <Text className="text-sm font-black text-brand">
              {shownAudience
                ? t(`visibility.${shownAudience.key}.label`)
                : tm("notes.defaultAudience")}{" "}
              ▾
            </Text>
          </Pressable>
          <TextInput
            testID="note-text"
            value={text}
            autoFocus
            multiline
            onChangeText={edit}
            onSelectionChange={(event) => setCaret(event.nativeEvent.selection.end)}
            placeholder={replyTo ? t("compose.replyPlaceholder") : t("compose.placeholder")}
            placeholderTextColor={colors["ink-faint"]}
            className="min-h-[120px] text-lg text-ink"
            textAlignVertical="top"
          />
          {hashtag && (suggestions.data?.length ?? 0) > 0 && (
            <View
              className="flex-row flex-wrap gap-2"
              accessibilityLabel={t("compose.suggestions")}
            >
              {suggestions.data!.map((suggestion) => (
                <Pressable
                  key={suggestion.tag}
                  accessibilityRole="button"
                  onPress={() => {
                    const done = applyCompletion(text, hashtag, `#${suggestion.tag}`);
                    edit(done.text);
                    setCaret(done.caret);
                  }}
                  className="rounded-full bg-brand-soft px-3 py-1.5"
                >
                  <Text className="font-bold text-brand-strong">#{suggestion.tag}</Text>
                </Pressable>
              ))}
            </View>
          )}
          {warning && (
            <TextInput
              testID="note-warning-text"
              value={warningText}
              onChangeText={(next) => {
                setWarningText(next);
                setTouched(true);
              }}
              maxLength={100}
              placeholder={t("compose.warningPlaceholder")}
              placeholderTextColor={colors["ink-faint"]}
              className="h-11 rounded-control bg-surface px-4 text-base text-ink"
            />
          )}
          {photos.photos.length > 0 && (
            <ScrollView horizontal contentContainerClassName="gap-2">
              {photos.photos.map((photo) => (
                <View key={photo.key} className="size-24 overflow-hidden rounded-2xl bg-surface-3">
                  <Image
                    source={photo.uri}
                    contentFit="cover"
                    style={{ width: "100%", height: "100%" }}
                  />
                  {!photo.assetId && !photo.error && (
                    <View className="absolute inset-0 items-center justify-center bg-black/40">
                      <Text className="font-black text-white">
                        {Math.round(photo.progress * 100)}%
                      </Text>
                    </View>
                  )}
                  {photo.error !== undefined && (
                    <View className="absolute inset-0 items-center justify-center bg-danger/70 px-1">
                      <Text className="text-center text-xs font-black text-white" numberOfLines={3}>
                        {errorMessage(photo.error, { namespace: "notes" })}
                      </Text>
                    </View>
                  )}
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel={t("uploads.remove")}
                    onPress={() => photos.remove(photo.key)}
                    className="absolute top-1 right-1 rounded-full bg-black/60 p-1"
                  >
                    <X color="#ffffff" size={14} />
                  </Pressable>
                </View>
              ))}
            </ScrollView>
          )}
          {poll && (
            <PollEditor
              poll={poll}
              onChange={(next) => {
                setPoll(next);
                setTouched(true);
              }}
              onRemove={() => setPoll(undefined)}
            />
          )}
          {quote && (
            <View className="gap-1 rounded-2xl border border-line p-3">
              <Text className="text-xs font-bold text-ink-soft">
                {quote.author?.displayName || quote.author?.handle}
              </Text>
              <NoteRichText note={quote} numberOfLines={3} />
            </View>
          )}
        </ScrollView>
        <View className="flex-row items-center gap-1 border-t border-line bg-surface px-2 py-2">
          <Pressable
            testID="note-add-photo"
            accessibilityRole="button"
            accessibilityLabel={t("compose.addMedia")}
            disabled={photos.photos.length >= 10}
            onPress={async () => {
              if ((await photos.pick()) === "denied") Alert.alert(tm("chat.photoDenied"));
              setTouched(true);
            }}
            className="rounded-full p-2.5"
          >
            <ImagePlus color={colors.brand} size={22} />
          </Pressable>
          <Pressable
            testID="note-add-poll"
            accessibilityRole="button"
            accessibilityLabel={t("compose.poll.title")}
            accessibilityState={{ selected: Boolean(poll) }}
            onPress={() => setPoll(poll ? undefined : newPoll())}
            className={`rounded-full p-2.5 ${poll ? "bg-brand-soft" : ""}`}
          >
            <BarChart3 color={colors.brand} size={22} />
          </Pressable>
          <Pressable
            testID="note-warning"
            accessibilityRole="switch"
            accessibilityLabel={t("compose.sensitive")}
            accessibilityState={{ checked: warning }}
            onPress={() => setWarning(!warning)}
            className={`rounded-full p-2.5 ${warning ? "bg-brand-soft" : ""}`}
          >
            <TriangleAlert color={warning ? colors.coral : colors.brand} size={22} />
          </Pressable>
          <Text
            className={`ml-auto mr-2 text-sm font-black ${over ? "text-danger" : "text-coral"}`}
          >
            {length > COUNT_FROM ? `${length}/${NOTE_MAX_CHARS}` : ""}
          </Text>
          <Button
            testID="note-save-draft"
            variant="ghost"
            size="sm"
            disabled={!hasContent || over || photos.busy || Boolean(busy)}
            loading={busy === "draft"}
            onPress={() => void keepAsDraft()}
          >
            {t("compose.saveDraft")}
          </Button>
          <Button
            testID="note-post"
            size="sm"
            disabled={!canPost}
            loading={busy === "post"}
            onPress={() => void post()}
          >
            {replyTo ? t("compose.reply") : t("compose.post")}
          </Button>
        </View>
      </KeyboardAvoidingView>
      {picking && (
        <AudienceSheet
          value={wanted}
          minRank={ceiling && ceiling.rank < 3 ? ceiling.rank : 0}
          onChange={(next) => {
            setChosen(next);
            setTouched(true);
          }}
          onClose={() => setPicking(false)}
        />
      )}
    </SafeAreaView>
  );
}
