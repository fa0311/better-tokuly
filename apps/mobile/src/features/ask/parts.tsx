import { ask, type common, engagement, toMillis } from "@tokuly/api";
import { charCount, hasEngagement, questionSubject } from "@tokuly/core/ask";
import { compactCount, type Subject } from "@tokuly/core/engagement";
import { compactAge } from "@tokuly/core/format";
import { Image } from "expo-image";
import { router } from "expo-router";
import { Heart, MessageCircle } from "lucide-react-native";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { ActionSheet } from "#/components/action-sheet";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { useEngagement, useLike } from "#/features/engagement/data";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { SERVER_URL } from "#/lib/config";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";

/** An API failure in this area's words. */
export const showAskError = (error: unknown): void =>
  Alert.alert(errorMessage(error, { namespace: "ask" }));

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** The smallest stored variant of an image that still covers `pixels` on a 3x screen. */
function imageSource(image: common.Image, pixels: number): string | undefined {
  const wanted = pixels * 3;
  const fitting = image.variants.filter((v) => v.size >= wanted).sort((a, b) => a.size - b.size)[0];
  const url = fitting?.url || image.url;
  return url ? absolute(url) : undefined;
}

/** A picture inside a question or an answer. */
export function Picture({ image }: { image: common.Image | undefined }) {
  const uri = image ? imageSource(image, 240) : undefined;
  if (!image || !uri) return null;
  const ratio = image.width && image.height ? image.width / image.height : 1;
  return (
    <Image
      source={{ uri }}
      accessibilityIgnoresInvertColors
      style={{
        width: 240,
        aspectRatio: Math.max(0.6, Math.min(ratio, 1.8)),
        borderRadius: 16,
        backgroundColor: image.dominantColor || undefined,
      }}
      contentFit="cover"
    />
  );
}

/** A stamp on an answer. */
function Stamp({ stamp }: { stamp: common.StampRef }) {
  const uri = stamp.animationUrl || stamp.imageUrl;
  if (!uri) return null;
  return (
    <Image
      source={{ uri: absolute(uri) }}
      accessibilityLabel={stamp.name}
      style={{ width: 120, height: 120 }}
      contentFit="contain"
    />
  );
}

/** Who sent something: "匿名さん", or the person when they signed. */
export function Sender({
  anonymous,
  user,
}: {
  anonymous: boolean;
  user: common.UserSummary | undefined;
}) {
  const { t } = useTranslation("ask");
  if (!anonymous && user)
    return (
      <Pressable
        testID="ask-sender"
        accessibilityRole="link"
        onPress={() => router.push(`/u/${user.handle}`)}
        className="min-w-0 shrink flex-row items-center gap-1.5"
      >
        <Avatar user={user} size="xs" />
        <Text numberOfLines={1} className="shrink text-sm font-black text-ink">
          {user.displayName}
        </Text>
      </Pressable>
    );
  return (
    <View testID="ask-sender" className="flex-row items-center gap-1.5">
      <View className="size-6 items-center justify-center rounded-md bg-surface-3">
        <Text className="text-xs">🙈</Text>
      </View>
      <Text className="text-sm font-black text-ink-soft">{t("anonymous")}</Text>
    </View>
  );
}

/** When a question arrived, as "3分前". */
export function Age({ at }: { at: ask.Question["createdAt"] }) {
  const { i18n } = useTranslation();
  return <Text className="text-xs font-bold text-ink-faint">{compactAge(i18n, toMillis(at))}</Text>;
}

/** The label of an answer that is not for everyone. */
export function AudienceBadge({ audience }: { audience: ask.AnswerAudience }) {
  const { t } = useTranslation("ask");
  const label =
    audience === ask.AnswerAudience.FRIENDS
      ? `👥 ${t("answer.badge.friends")}`
      : audience === ask.AnswerAudience.PRIVATE
        ? `🔒 ${t("answer.badge.private")}`
        : null;
  if (!label) return null;
  return (
    <View
      className={`rounded-full px-2 py-0.5 ${audience === ask.AnswerAudience.FRIENDS ? "bg-sky" : "bg-sun"}`}
    >
      <Text className="text-[11px] font-black" style={{ color: INK_ON_BRIGHT }}>
        {label}
      </Text>
    </View>
  );
}

/** "12 / 300", turning red past the limit. */
export function Counter({ text, max }: { text: string; max: number }) {
  const count = charCount(text);
  return (
    <Text className={`text-xs font-bold ${count > max ? "text-danger" : "text-ink-faint"}`}>
      {`${count} / ${max}`}
    </Text>
  );
}

/** The like of an answer on the Q&A page, with its count. */
function LikeButton({ subject }: { subject: Subject }) {
  const { t, i18n } = useTranslation("ask");
  const { colors } = useTheme();
  const like = useLike(subject);
  const { data } = useEngagement(subject);
  const liked = data?.viewerReaction?.kind === engagement.ReactionKind.LIKE;
  const count = data?.likeCount ?? 0;
  return (
    <Pressable
      testID="ask-like"
      accessibilityRole="button"
      accessibilityLabel={t("question.like", { count })}
      accessibilityState={{ selected: liked }}
      onPress={() => void like(!liked)}
      className={`h-9 flex-row items-center gap-1.5 rounded-full px-3 ${liked ? "bg-pink/15" : ""}`}
    >
      <Heart
        size={18}
        color={liked ? colors.pink : colors["ink-soft"]}
        fill={liked ? colors.pink : "transparent"}
      />
      {count > 0 && (
        <Text className="text-sm font-black text-ink-soft">
          {compactCount(count, i18n.language)}
        </Text>
      )}
    </Pressable>
  );
}

/**
 * One answered question as a card: the question on top (who asked, when),
 * the owner's answer below, and the like and comment count where the answer
 * is on the Q&A page. A tap on the question opens its page.
 */
export function QaCard({
  question,
  link = true,
  extra,
}: {
  question: ask.Question;
  /** The question leads to its own page. */
  link?: boolean;
  /** More controls at the end of the footer (the owner's menu). */
  extra?: ReactNode;
}) {
  const { t, i18n } = useTranslation("ask");
  const { colors } = useTheme();
  const answer = question.answer;
  const text = (
    <Text className="mt-1.5 text-lg leading-6 font-black text-ink">{question.text}</Text>
  );
  return (
    <View
      testID="ask-qa-card"
      className="overflow-hidden rounded-card bg-surface"
      style={sticker(colors)}
    >
      <View className="gap-2 bg-brand-soft px-4 pt-3.5 pb-4">
        <View className="flex-row items-center gap-2">
          <Sender anonymous={question.anonymous} user={question.asker} />
          <View className="flex-1" />
          <Age at={question.createdAt} />
        </View>
        {link ? (
          <Pressable accessibilityRole="link" onPress={() => router.push(`/ask/q/${question.id}`)}>
            {text}
          </Pressable>
        ) : (
          text
        )}
        <Picture image={question.image} />
      </View>
      {answer && (
        <View className="flex-row gap-3 border-t-2 border-ink px-4 pt-3.5 pb-2">
          <Avatar user={question.owner} size="sm" />
          <View className="min-w-0 flex-1 gap-2">
            {answer.text ? (
              <Text testID="ask-answer" className="text-base leading-6 text-ink">
                {answer.text}
              </Text>
            ) : null}
            <Picture image={answer.image} />
            {answer.stamp && <Stamp stamp={answer.stamp} />}
            <View className="min-h-9 flex-row flex-wrap items-center gap-1">
              {hasEngagement(question) && (
                <>
                  <LikeButton subject={questionSubject(question.id)} />
                  <View
                    accessibilityLabel={t("question.commentCount", {
                      count: question.commentCount,
                    })}
                    className="h-9 flex-row items-center gap-1.5 px-2"
                  >
                    <MessageCircle size={18} color={colors["ink-soft"]} />
                    {question.commentCount > 0 && (
                      <Text className="text-sm font-black text-ink-soft">
                        {compactCount(question.commentCount, i18n.language)}
                      </Text>
                    )}
                  </View>
                </>
              )}
              <AudienceBadge audience={answer.audience} />
              <View className="flex-1" />
              {answer.editedAt && (
                <Text className="text-xs font-bold text-ink-faint">{t("answer.edited")}</Text>
              )}
              {extra}
            </View>
          </View>
        </View>
      )}
    </View>
  );
}

const REPORT_CATEGORIES = [
  ["SPAM", engagement.ReportCategory.SPAM],
  ["HARASSMENT", engagement.ReportCategory.HARASSMENT],
  ["HATE", engagement.ReportCategory.HATE],
  ["SEXUAL", engagement.ReportCategory.SEXUAL],
  ["VIOLENCE", engagement.ReportCategory.VIOLENCE],
  ["SELF_HARM", engagement.ReportCategory.SELF_HARM],
  ["ILLEGAL", engagement.ReportCategory.ILLEGAL],
  ["OTHER", engagement.ReportCategory.OTHER],
] as const;

/**
 * Reporting a question (with its answer): the reasons rise from the bottom.
 * The server decides who is told what; an anonymous sender stays anonymous
 * to the one who reports. The report is made as the account in use when the
 * reason was tapped, and only that account hears how it went.
 */
export function ReportSheet({
  questionId,
  onClose,
}: {
  /** The question reported; `null` closes the sheet. */
  questionId: string | null;
  onClose(): void;
}) {
  const { t } = useTranslation("mobile");
  const send = async (category: engagement.ReportCategory) => {
    if (!questionId) return;
    const account = currentAccount();
    const reports = (account?.api ?? api()).service(engagement.ReportService);
    const mine = () => account !== null && isActive(account);
    try {
      await reports.createReport({ subject: questionSubject(questionId), category, text: "" });
      if (mine()) Alert.alert(t("ask.report.sent"));
    } catch (error) {
      if (mine()) showAskError(error);
    }
  };
  return (
    <ActionSheet
      visible={questionId !== null}
      title={t("ask.report.title")}
      onClose={onClose}
      actions={REPORT_CATEGORIES.map(([key, category]) => ({
        key: `report-${key}`,
        label: t(`ask.report.categories.${key}`),
        onPress: () => void send(category),
      }))}
    />
  );
}

/** "もっと見る" under a list that has more pages. */
export function MoreButton({
  query,
}: {
  query: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage(): unknown };
}) {
  const { t } = useTranslation("mobile");
  if (!query.hasNextPage) return null;
  return (
    <View className="items-center">
      <Button
        variant="ghost"
        loading={query.isFetchingNextPage}
        onPress={() => void query.fetchNextPage()}
      >
        {t("ask.more")}
      </Button>
    </View>
  );
}
