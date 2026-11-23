import { Link } from "@tanstack/react-router";
import { Play } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { TextField } from "#/components/ui/field";
import { cn } from "#/lib/cn";
import { Notice } from "../parts";
import { CodeBlock } from "./code-block";
import { methodUrl } from "./data";

/** What came back from a call made by the panel. */
interface Answer {
  status: number;
  statusText: string;
  milliseconds: number;
  /** The headers worth showing: the error reason, the missing scope, the rate limit. */
  headers: [name: string, value: string][];
  body: string;
  json: boolean;
}

const SHOWN_HEADERS = [
  "x-tokuly-error",
  "x-tokuly-scope",
  "x-ratelimit-limit",
  "x-ratelimit-remaining",
  "x-ratelimit-reset",
  "retry-after",
];

/**
 * Calls a method for real, with a token the person pastes in. The call goes
 * out without cookies, so it is exactly what a script holding that token
 * would get: the browser's own session plays no part.
 */
export function TryIt({ path, initialBody }: { path: string; initialBody: string }) {
  const { t } = useTranslation("dev");
  // The pasted token lives in this panel only: it is gone when the panel is (another method,
  // another page, signing out), and never reaches browser storage or a shared store.
  const [token, setToken] = useState("");
  const [body, setBody] = useState(initialBody);
  const [answer, setAnswer] = useState<Answer | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const send = async (event: FormEvent) => {
    event.preventDefault();
    setProblem(null);
    try {
      JSON.parse(body);
    } catch {
      setProblem(t("reference.tryIt.invalidJson"));
      return;
    }
    setBusy(true);
    const started = performance.now();
    try {
      const response = await fetch(methodUrl(path), {
        method: "POST",
        credentials: "omit",
        headers: {
          "content-type": "application/json",
          ...(token.trim() ? { authorization: `Bearer ${token.trim()}` } : {}),
        },
        body,
      });
      const text = await response.text();
      let formatted = text;
      let json = false;
      try {
        formatted = JSON.stringify(JSON.parse(text), null, 2);
        json = true;
      } catch {
        // Not JSON (a proxy's error page): shown as it came.
      }
      setAnswer({
        status: response.status,
        statusText: response.statusText,
        milliseconds: Math.round(performance.now() - started),
        headers: SHOWN_HEADERS.flatMap((name): [string, string][] => {
          const value = response.headers.get(name);
          return value ? [[name, value]] : [];
        }),
        body: formatted,
        json,
      });
    } catch {
      setAnswer(null);
      setProblem(t("reference.tryIt.failed"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section
      aria-label={t("reference.tryIt.title")}
      className="sticker rounded-card bg-surface p-4"
    >
      <h2 className="text-lg font-black">{t("reference.tryIt.title")}</h2>
      <p className="mt-1 text-xs leading-relaxed text-ink-soft">{t("reference.tryIt.lead")}</p>
      <form onSubmit={send} className="mt-3 flex flex-col gap-3">
        <TextField
          label={t("reference.tryIt.token")}
          type="password"
          value={token}
          onChange={(event) => setToken(event.target.value)}
          placeholder="tkl_pat_…"
          autoComplete="off"
          spellCheck={false}
          hint={
            <>
              {t("reference.tryIt.tokenHint")}{" "}
              <Link
                to="/developers/tokens"
                className="font-bold text-brand underline-offset-2 hover:underline"
              >
                {t("reference.tryIt.makeToken")}
              </Link>
            </>
          }
        />
        <label className="flex flex-col gap-1.5">
          <span className="px-1 text-sm font-bold text-ink-soft">{t("reference.tryIt.body")}</span>
          <textarea
            value={body}
            onChange={(event) => setBody(event.target.value)}
            spellCheck={false}
            autoCapitalize="none"
            rows={Math.min(16, Math.max(4, body.split("\n").length))}
            className="w-full resize-y rounded-control border-2 border-line bg-surface px-3 py-2.5 font-mono text-[12.5px] leading-relaxed focus:border-brand focus:outline-none"
          />
        </label>
        {problem && <Notice tone="danger">{problem}</Notice>}
        <Button type="submit" loading={busy} icon={<Play className="size-4" />}>
          {t("reference.tryIt.send")}
        </Button>
      </form>

      {answer && (
        <section className="mt-4 flex flex-col gap-2" aria-label={t("reference.tryIt.answer")}>
          <p className="flex flex-wrap items-center gap-2">
            <Chip
              className={cn(
                "font-mono",
                answer.status < 300 ? "bg-lime text-[#17122b]" : "bg-danger/15 text-danger",
              )}
            >
              {answer.status} {answer.statusText}
            </Chip>
            <span className="text-xs font-bold text-ink-soft tabular-nums">
              {answer.milliseconds} ms
            </span>
          </p>
          {answer.headers.length > 0 && (
            <dl className="rounded-2xl bg-surface-2 px-3 py-2 font-mono text-xs">
              {answer.headers.map(([name, value]) => (
                <div key={name} className="flex gap-2">
                  <dt className="text-ink-soft">{name}:</dt>
                  <dd className="min-w-0 font-bold break-all">{value}</dd>
                </div>
              ))}
            </dl>
          )}
          <CodeBlock
            code={answer.body || t("reference.tryIt.emptyBody")}
            language={answer.json ? "json" : "text"}
            label={t("reference.tryIt.answer")}
          />
        </section>
      )}
    </section>
  );
}
