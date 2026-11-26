import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { community, errorReason } from "@tokuly/api";
import {
  communityQueries,
  imageUrl,
  inviteHref,
  refreshMembership,
  storeCommunity,
} from "@tokuly/core/community";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Logo } from "#/components/ui/logo";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useSessionStatus } from "#/stores/session";
import { communityClient, NS } from "./api";
import { CommunityIcon, categoryOf, MemberCount, VisibilityChip } from "./parts";

/**
 * What an invite link opens: the community it leads to, and one button.
 * Someone without an account is sent through sign-up (or sign-in) and comes
 * back here; someone signed in joins at once.
 */
export function InviteScreen({ code }: { code: string }) {
  const { t } = useTranslation(NS);
  const status = useSessionStatus();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const here = inviteHref(code);
  const preview = useQuery({
    ...communityQueries.invitePreview(communityClient, code),
    // The preview names the viewer's own standing once they are signed in.
    enabled: status !== "unknown",
  });
  const join = useMutation({
    mutationFn: () => communityClient().joinCommunity({ inviteCode: code }),
    onSuccess: ({ community: joined }) => {
      if (joined) storeCommunity(queryClient, joined);
      refreshMembership(queryClient, joined?.id);
      toast.success(t("join.welcome"), { icon: "🎉" });
      if (joined) void navigate({ to: "/g/$handle", params: { handle: joined.handle } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });

  const c = preview.data?.community;
  const inviter = preview.data?.inviter;
  const banner = imageUrl(c?.banner, 640);
  const member = c?.viewer?.status === community.MembershipStatus.MEMBER;
  const banned = c?.viewer?.status === community.MembershipStatus.BANNED;

  return (
    <main className="min-h-dvh bg-bg">
      <header className="mx-auto flex h-16 max-w-md items-center px-4">
        <Link to="/" aria-label="Tokuly">
          <Logo />
        </Link>
      </header>
      <div className="mx-auto max-w-md px-4 pb-16">
        {status === "unknown" || preview.isPending ? (
          <PaneSpinner />
        ) : !c ? (
          <EmptyState
            emoji="💔"
            title={
              errorReason(preview.error) === "INVITE_INVALID"
                ? t("invite.invalid")
                : errorMessage(preview.error, { namespace: NS })
            }
            body={t("invite.invalidBody")}
            action={
              <Link to="/">
                <Button variant="secondary">{t("invite.home")}</Button>
              </Link>
            }
          />
        ) : (
          <article className="sticker animate-pop-in overflow-hidden rounded-card bg-surface">
            <div
              className="h-28 bg-brand-soft"
              style={
                banner
                  ? {
                      backgroundImage: `url("${banner}")`,
                      backgroundSize: "cover",
                      backgroundPosition: "center",
                    }
                  : undefined
              }
            />
            <div className="px-5 pb-6">
              <div className="-mt-10">
                <CommunityIcon community={c} size="lg" />
              </div>
              {inviter && (
                <p className="mt-3 flex items-center gap-2 text-sm text-ink-soft">
                  <Avatar user={inviter} size="xs" />
                  <span className="min-w-0 truncate">
                    {t("invite.from", { name: inviter.displayName || inviter.handle })}
                  </span>
                </p>
              )}
              <h1 className="mt-1 break-words text-2xl font-black leading-tight">
                <EmojiText text={c.name} />
              </h1>
              <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1.5">
                <Chip>
                  <span aria-hidden>{categoryOf(c.category).emoji}</span>
                  {t(`category.${categoryOf(c.category).key}`)}
                </Chip>
                <VisibilityChip visibility={c.visibility} />
                <MemberCount count={c.memberCount} />
              </div>
              {c.description && (
                <p className="mt-3 whitespace-pre-wrap break-words leading-relaxed">
                  <EmojiText text={c.description} />
                </p>
              )}
              {c.rules.length > 0 && (
                <ol className="mt-4 flex list-decimal flex-col gap-1 pl-5 text-sm text-ink-soft">
                  {c.rules.map((rule, index) => (
                    <li key={index}>{rule.title}</li>
                  ))}
                </ol>
              )}
              <div className="mt-6 flex flex-col gap-2">
                {status !== "signedIn" ? (
                  <>
                    <Link to="/signup" search={{ redirect: here }}>
                      <Button block size="lg">
                        {t("invite.signUp")}
                      </Button>
                    </Link>
                    <Link to="/login" search={{ redirect: here }}>
                      <Button block variant="secondary">
                        {t("invite.signIn")}
                      </Button>
                    </Link>
                  </>
                ) : member ? (
                  <Link to="/g/$handle" params={{ handle: c.handle }}>
                    <Button block size="lg" variant="lime">
                      {t("invite.open")}
                    </Button>
                  </Link>
                ) : banned ? (
                  <p className="rounded-2xl bg-sun px-4 py-3 text-sm font-bold text-[#17122b]">
                    {t("ban.you")}
                  </p>
                ) : (
                  <Button block size="lg" loading={join.isPending} onClick={() => join.mutate()}>
                    {t("invite.join")}
                  </Button>
                )}
              </div>
            </div>
          </article>
        )}
      </div>
    </main>
  );
}
