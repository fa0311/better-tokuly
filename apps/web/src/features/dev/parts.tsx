import { type Timestamp, toMillis } from "@tokuly/api";
import { Check, Copy, TriangleAlert } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button, type ButtonProps } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { useAccountTimeZone } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import i18n from "#/lib/i18n";

/** Puts text on the clipboard; `false` when the browser refuses. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** A button that copies `text` and says so for a moment. */
export function CopyButton({
  text,
  label,
  ...props
}: { text: string; label?: string } & Omit<ButtonProps, "onClick" | "icon">) {
  const { t } = useTranslation("dev");
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    if (!(await copyText(text))) {
      toast.error(t("copy.failed"));
      return;
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };
  return (
    <Button
      variant="secondary"
      size="sm"
      {...props}
      onClick={copy}
      icon={copied ? <Check className="size-4" /> : <Copy className="size-4" />}
    >
      {copied ? t("copy.done") : (label ?? t("copy.action"))}
    </Button>
  );
}

/** A secret that was just made, with what it is for. */
export interface RevealedSecret {
  /** What kind of secret this is, as the dialog's title. */
  title: string;
  /** What to do with it. */
  lead: string;
  secret: string;
}

/**
 * Shows a token, client secret or signing secret exactly once.
 *
 * The secret lives in the state of the screen that made it and nowhere else:
 * it is not written to the query cache or to any browser storage, and closing
 * the dialog drops it (the caller clears its state in `onClose`).
 */
export function SecretDialog({
  revealed,
  onClose,
}: {
  revealed: RevealedSecret | null;
  onClose(): void;
}) {
  const { t } = useTranslation("dev");
  return (
    <Dialog
      open={revealed !== null}
      onOpenChange={(open) => !open && onClose()}
      title={revealed?.title ?? ""}
      description={revealed?.lead}
      footer={<Button onClick={onClose}>{t("secret.saved")}</Button>}
    >
      {revealed && (
        <div className="flex flex-col gap-3">
          <Notice tone="warning">{t("secret.once")}</Notice>
          <input
            readOnly
            aria-label={t("secret.label")}
            value={revealed.secret}
            onFocus={(event) => event.target.select()}
            autoComplete="off"
            spellCheck={false}
            className="h-12 w-full rounded-control border-2 border-ink bg-surface-2 px-3 font-mono text-sm"
          />
          <CopyButton text={revealed.secret} size="md" variant="lime" />
        </div>
      )}
    </Dialog>
  );
}

/**
 * Runs something that returns a secret. Unlike a mutation, nothing of the
 * answer is kept anywhere: it is handed to `onDone` and forgotten.
 */
export function useSecretAction<Args, Result>(
  run: (args: Args) => Promise<Result>,
  onDone: (result: Result) => void,
): { run(args: Args): Promise<void>; busy: boolean } {
  const [busy, setBusy] = useState(false);
  return {
    busy,
    run: async (args) => {
      setBusy(true);
      try {
        onDone(await run(args));
      } catch (error) {
        toast.error(errorMessage(error, { namespace: "dev" }));
      } finally {
        setBusy(false);
      }
    },
  };
}

/** A boxed remark: `warning` for things to be careful with, `danger` for what went wrong. */
export function Notice({
  tone = "info",
  children,
  className,
}: {
  tone?: "info" | "warning" | "danger";
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex gap-2 rounded-2xl px-3.5 py-3 text-sm font-bold leading-relaxed",
        tone === "info" && "bg-surface-2 text-ink-soft",
        tone === "warning" && "border-2 border-ink bg-sun text-[#17122b]",
        tone === "danger" && "bg-danger/10 text-danger",
        className,
      )}
    >
      {tone !== "info" && <TriangleAlert className="mt-0.5 size-4 shrink-0" />}
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Scopes as small monospace labels. */
export function ScopeChips({ scopes, className }: { scopes: string[]; className?: string }) {
  return (
    <span className={cn("flex flex-wrap gap-1", className)}>
      {scopes.map((scope) => (
        <Chip key={scope} className="font-mono">
          {scope}
        </Chip>
      ))}
    </span>
  );
}

/** A section of a detail page: a heading, an optional control beside it, the content. */
export function Section({
  title,
  action,
  children,
  className,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("mt-7 first:mt-0", className)}>
      <div className="mb-2.5 flex min-h-9 items-center gap-3 px-1">
        <h2 className="min-w-0 flex-1 text-lg font-black">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

/** A label and its value on one line of a facts list. */
export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 py-1.5">
      <dt className="w-32 shrink-0 text-sm font-bold text-ink-soft">{label}</dt>
      <dd className="min-w-0 flex-1 break-words text-sm font-normal">{children}</dd>
    </div>
  );
}

/** A date and time in the account's time zone. */
export function formatDateTime(at: Timestamp | undefined, zone: string): string {
  if (!at) return "";
  return new Intl.DateTimeFormat(i18n.language, {
    timeZone: zone,
    dateStyle: "medium",
    timeStyle: "short",
  }).format(toMillis(at));
}

/** A moment as the account's clock shows it, with how long ago it was as the tooltip. */
export function When({ at }: { at: Timestamp | undefined }) {
  const zone = useAccountTimeZone();
  if (!at) return null;
  return <time title={compactAge(toMillis(at))}>{formatDateTime(at, zone)}</time>;
}

/** How long ago something happened ("3時間前"), with the exact time as the tooltip. */
export function Ago({ at }: { at: Timestamp | undefined }) {
  const zone = useAccountTimeZone();
  if (!at) return null;
  return <time title={formatDateTime(at, zone)}>{compactAge(toMillis(at))}</time>;
}
