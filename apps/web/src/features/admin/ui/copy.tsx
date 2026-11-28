import { Check, Copy } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "#/lib/use-translation";

/** An ID in monospace with a button that copies it. */
export function CopyableId({ value }: { value: string }) {
  const { t } = useTranslation("common");
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <span className="inline-flex max-w-full items-center gap-1 align-middle">
      <code className="min-w-0 truncate rounded-md bg-surface-2 px-1.5 py-0.5 font-mono text-xs">
        {value}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label={copied ? t("actions.copied") : t("actions.copy")}
        className="grid size-7 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
      >
        {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
      </button>
    </span>
  );
}
