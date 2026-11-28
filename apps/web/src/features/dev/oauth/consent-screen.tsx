import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { authorizationClient, type dev } from "../api";
import { Notice } from "../parts";
import { AnswerButtons, ConsentDetails, OAuthPage } from "./parts";
import {
  type AuthorizationParams,
  authorizationParams,
  carriedQuery,
  carryQuery,
  leaveFor,
} from "./redirect";

/**
 * `/oauth/authorize`: the authorization endpoint of OAuth. An application
 * sends the browser here; the person sees who is asking and for what, and
 * allows or refuses.
 *
 * Rules this page keeps:
 * - It never approves by itself. `ApproveAuthorization` is called from the
 *   allow button's click handler and from nowhere else; nothing in the
 *   address can press it, and it is asked again on every visit, also when
 *   the server says the same access was given before.
 * - It only ever leaves for the `redirect_url` the server returns.
 * - It does not render inside a frame (and the route says so in its response
 *   headers).
 */
export function ConsentScreen() {
  const { t } = useTranslation("dev");
  // Read once, from the address itself: the request as the application wrote it.
  const [search] = useState(() => window.location.search);
  const request = authorizationParams(carriedQuery(search));
  return (
    <OAuthPage
      title={t("oauth.consent.title")}
      returnTo={`/oauth/authorize?request=${carryQuery(carriedQuery(search).toString())}`}
    >
      <ConsentRequest request={request} />
    </OAuthPage>
  );
}

function ConsentRequest({ request }: { request: AuthorizationParams }) {
  const { t } = useTranslation("dev");
  const [busy, setBusy] = useState<"approve" | "deny" | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [leaving, setLeaving] = useState(false);

  // Describing the request changes nothing on the server. Never cached: the
  // answer belongs to this visit.
  const consent = useQuery({
    queryKey: ["dev", "consent", request],
    queryFn: () => authorizationClient().getConsent({ request }),
    retry: false,
    gcTime: 0,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const leave = (redirectUrl: string) => {
    if (leaveFor(redirectUrl)) setLeaving(true);
    else setProblem(t("oauth.consent.badRedirect"));
  };

  /** The person pressed a button: tell the server, then go where it says. */
  const answer = async (choice: "approve" | "deny") => {
    setBusy(choice);
    setProblem(null);
    try {
      const response =
        choice === "approve"
          ? await authorizationClient().approveAuthorization({ request })
          : await authorizationClient().denyAuthorization({ request });
      leave(response.redirectUrl);
    } catch (error) {
      setProblem(errorMessage(error, { namespace: "dev" }));
    } finally {
      setBusy(null);
    }
  };

  if (leaving) return <p className="py-6 text-center font-bold">{t("oauth.consent.leaving")}</p>;
  if (consent.isPending) return <Skeleton className="h-72" />;
  // No usable application, or a redirect URI it never registered: say so here.
  // There is nowhere safe to send the browser.
  if (consent.isError)
    return (
      <div className="flex flex-col gap-3">
        <Notice tone="danger">{errorMessage(consent.error, { namespace: "dev" })}</Notice>
        <p className="text-sm leading-relaxed text-ink-soft">{t("oauth.consent.invalidBody")}</p>
      </div>
    );

  const result = consent.data.result;
  // The request cannot be granted and the application is to be told why. The
  // person decides when to go back; nothing was approved.
  if (result.case === "redirectUrl")
    return (
      <div className="flex flex-col gap-4">
        <Notice tone="danger">{t("oauth.consent.rejected")}</Notice>
        {problem && <Notice tone="danger">{problem}</Notice>}
        <Button block onClick={() => leave(result.value)}>
          {t("oauth.consent.backToApp")}
        </Button>
      </div>
    );
  if (result.case !== "consent")
    return <Notice tone="danger">{t("oauth.consent.invalidBody")}</Notice>;

  const details: dev.Consent = result.value;
  return (
    <div className="flex flex-col gap-5">
      <ConsentDetails consent={details} />
      <p className="text-xs leading-relaxed text-ink-soft">
        {t("oauth.consent.redirectHost", { host: details.redirectHost })}
      </p>
      {problem && <Notice tone="danger">{problem}</Notice>}
      <AnswerButtons busy={busy} onAnswer={(choice) => void answer(choice)} />
      <p className="text-center text-xs leading-relaxed text-ink-faint">
        {t("oauth.consent.revokeHint")}
      </p>
    </div>
  );
}
