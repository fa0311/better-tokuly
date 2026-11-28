import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Button } from "#/components/ui/button";
import { TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { authorizationClient } from "../api";
import { Notice } from "../parts";
import { AnswerButtons, ConsentDetails, OAuthPage } from "./parts";

/**
 * `/oauth/device`: the device flow (RFC 8628). A program without a browser
 * shows a code; here the person types it, sees which application is asking
 * and for what, and lets it in or turns it away.
 *
 * A code in the address (`?user_code=`) only fills the field. Looking it up
 * and approving are both things the person presses a button for: this is
 * the flow phishing uses, so the page asks them to compare the code with
 * the one on their own device first.
 */
export function DeviceScreen() {
  const { t } = useTranslation("dev");
  // Read from the address itself: the router would turn a code of digits into a number.
  const [userCode] = useState(() => new URLSearchParams(window.location.search).get("user_code"));
  const [typed, setTyped] = useState(userCode ?? "");
  const [code, setCode] = useState<string>();
  const returnTo = userCode
    ? `/oauth/device?user_code=${encodeURIComponent(userCode)}`
    : "/oauth/device";
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (typed.trim()) setCode(typed.trim());
  };
  return (
    <OAuthPage title={t("oauth.device.title")} returnTo={returnTo}>
      {code ? (
        <DeviceRequest key={code} code={code} onOtherCode={() => setCode(undefined)} />
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-4">
          <p className="text-sm leading-relaxed text-ink-soft">{t("oauth.device.lead")}</p>
          <TextField
            label={t("oauth.device.code")}
            value={typed}
            onChange={(event) => setTyped(event.target.value.toUpperCase())}
            placeholder="ABCD-EFGH"
            autoCapitalize="characters"
            autoComplete="off"
            spellCheck={false}
            maxLength={16}
            required
          />
          <Button type="submit" size="lg" block disabled={!typed.trim()}>
            {t("oauth.device.check")}
          </Button>
        </form>
      )}
    </OAuthPage>
  );
}

function DeviceRequest({ code, onOtherCode }: { code: string; onOtherCode(): void }) {
  const { t } = useTranslation("dev");
  const [busy, setBusy] = useState<"approve" | "deny" | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [answered, setAnswered] = useState<"approved" | "denied" | null>(null);

  const consent = useQuery({
    queryKey: ["dev", "device-consent", code],
    queryFn: async () => (await authorizationClient().getDeviceConsent({ userCode: code })).consent,
    retry: false,
    gcTime: 0,
    staleTime: Number.POSITIVE_INFINITY,
  });

  const answer = async (choice: "approve" | "deny") => {
    setBusy(choice);
    setProblem(null);
    try {
      if (choice === "approve") await authorizationClient().approveDevice({ userCode: code });
      else await authorizationClient().denyDevice({ userCode: code });
      setAnswered(choice === "approve" ? "approved" : "denied");
    } catch (error) {
      setProblem(errorMessage(error, { namespace: "dev" }));
    } finally {
      setBusy(null);
    }
  };

  const another = (
    <Button variant="secondary" onClick={onOtherCode}>
      {t("oauth.device.otherCode")}
    </Button>
  );
  if (answered === "approved")
    return (
      <EmptyState
        emoji="🎉"
        title={t("oauth.device.approved")}
        body={t("oauth.device.approvedBody")}
      />
    );
  if (answered === "denied")
    return (
      <EmptyState
        emoji="🙅"
        title={t("oauth.device.denied")}
        body={t("oauth.device.deniedBody")}
        action={another}
      />
    );
  if (consent.isPending) return <Skeleton className="h-72" />;
  if (consent.isError || !consent.data)
    return (
      <EmptyState
        emoji="🤔"
        title={errorMessage(consent.error, { namespace: "dev" })}
        body={t("oauth.device.invalidBody")}
        action={another}
      />
    );

  return (
    <div className="flex flex-col gap-5">
      <Notice tone="warning">
        {t("oauth.device.compare")}
        <span className="mt-1 block font-mono text-lg tracking-widest">{code}</span>
      </Notice>
      <ConsentDetails consent={consent.data} />
      {problem && <Notice tone="danger">{problem}</Notice>}
      <AnswerButtons busy={busy} onAnswer={(choice) => void answer(choice)} />
    </div>
  );
}
