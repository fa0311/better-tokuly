import { useQuery } from "@tanstack/react-query";
import { Link, Outlet } from "@tanstack/react-router";
import { ChevronLeft, Search, X } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Chip } from "#/components/ui/card";
import { LogoMark } from "#/components/ui/logo";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { API_PATH } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { dev, eventTypesQuery, localized, scopesQuery } from "../api";
import { CopyButton, Notice } from "../parts";
import { splitScopes } from "../scopes";
import { CodeBlock } from "./code-block";
import {
  byDomain,
  methodPath,
  methodUrl,
  searchMethods,
  useMethod,
  useReference,
  useServices,
} from "./data";
import { curlExample, typescriptExample, WEBHOOK_VERIFY_EXAMPLE } from "./examples";
import { serviceShortName } from "./head";
import { exampleOf, parseSchema, reasonsOf } from "./schema";
import { Prose, SchemaView } from "./schema-view";
import { TryIt } from "./try-it";

/**
 * `/docs/api`: the public API reference. Everything on it comes from the
 * server (`ReferenceService`, the scope catalogue, the webhook event
 * catalogue), so it cannot disagree with the API it describes.
 *
 * The frame: a header, the list of services beside the page on wide screens
 * (on phones the list is part of the start page), and the open page.
 */
export function ReferenceLayout() {
  const { t } = useTranslation("dev");
  return (
    <div className="flex h-dvh flex-col bg-bg">
      <header className="glass z-20 flex h-16 shrink-0 items-center gap-3 border-b-2 border-line px-4">
        <Link to="/docs/api" className="flex min-w-0 items-center gap-2 text-lg font-black">
          <LogoMark className="size-8 shrink-0" />
          <span className="truncate">{t("reference.title")}</span>
        </Link>
        <span className="flex-1" />
        <Link
          to="/developers"
          className="pressable sticker-sm shrink-0 rounded-full bg-surface px-3.5 py-1.5 text-sm font-bold"
        >
          {t("reference.toConsole")}
        </Link>
      </header>
      <div className="flex min-h-0 flex-1">
        <aside className="hidden w-80 shrink-0 flex-col border-r-2 border-line lg:flex">
          <ServiceBrowser className="min-h-0 flex-1 overflow-y-auto p-3" />
        </aside>
        <main className="min-w-0 flex-1 overflow-y-auto">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

/** The search box and what it finds, or every service by domain. */
function ServiceBrowser({ className }: { className?: string }) {
  const { t } = useTranslation("dev");
  const services = useServices();
  const query = useReference((state) => state.query);
  const setQuery = useReference((state) => state.setQuery);

  return (
    <nav aria-label={t("reference.services")} className={className}>
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-3.5 size-4 -translate-y-1/2 text-ink-faint" />
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("reference.search")}
          aria-label={t("reference.search")}
          autoCapitalize="none"
          spellCheck={false}
          className="h-11 w-full rounded-full border-2 border-line bg-surface pr-10 pl-10 text-sm font-bold placeholder:font-medium placeholder:text-ink-faint focus:border-brand focus:outline-none [&::-webkit-search-cancel-button]:hidden"
        />
        {query && (
          <button
            type="button"
            aria-label={t("reference.clearSearch")}
            onClick={() => setQuery("")}
            className="absolute top-1/2 right-2 grid size-7 -translate-y-1/2 place-items-center rounded-full hover:bg-surface-2"
          >
            <X className="size-4" />
          </button>
        )}
      </div>
      <div className="mt-3">
        {services.isPending ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-8" />
            <Skeleton className="h-8" />
            <Skeleton className="h-8" />
          </div>
        ) : services.isError ? (
          <Notice tone="danger">{errorMessage(services.error, { namespace: "dev" })}</Notice>
        ) : query.trim() ? (
          <SearchResults services={services.data} query={query} />
        ) : (
          <ServiceTree services={services.data} />
        )}
      </div>
    </nav>
  );
}

const methodLink =
  "block truncate rounded-xl px-2.5 py-1.5 font-mono text-[13px] hover:bg-surface-2 data-[status=active]:bg-brand-soft data-[status=active]:font-bold";

function SearchResults({ services, query }: { services: dev.ApiService[]; query: string }) {
  const { t } = useTranslation("dev");
  const found = searchMethods(
    services.flatMap((service) => service.methods),
    query,
  );
  if (found.length === 0)
    return (
      <p className="px-2 py-6 text-center text-sm text-ink-soft">{t("reference.noResults")}</p>
    );
  return (
    <ul>
      {found.map((method) => (
        <li key={method.path}>
          <Link
            to="/docs/api/$service/$method"
            params={{ service: method.service, method: method.name }}
            className="block rounded-xl px-2.5 py-1.5 hover:bg-surface-2 data-[status=active]:bg-brand-soft"
          >
            <span className="block truncate font-mono text-[13px] font-bold">{method.name}</span>
            <span className="block truncate font-mono text-[11px] text-ink-soft">
              {serviceShortName(method.service)}
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function ServiceTree({ services }: { services: dev.ApiService[] }) {
  return (
    <ul className="flex flex-col gap-3">
      {byDomain(services).map((group) => (
        <li key={group.domain}>
          <p className="px-2.5 font-mono text-[11px] font-black tracking-wider text-ink-faint uppercase">
            {group.domain}
          </p>
          <ul>
            {group.services.map((service) => (
              <li key={service.name}>
                <ServiceBranch service={service} />
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ul>
  );
}

/** One service: its name leads to its page; its methods fold out below it. */
function ServiceBranch({ service }: { service: dev.ApiService }) {
  return (
    <details className="group" name="reference-service">
      <summary className="flex cursor-pointer list-none items-center gap-1 rounded-xl px-2.5 py-1.5 text-sm font-bold hover:bg-surface-2 [&::-webkit-details-marker]:hidden">
        <ChevronLeft className="size-4 shrink-0 -rotate-180 text-ink-faint transition-transform group-open:-rotate-90" />
        <span className="min-w-0 flex-1 truncate">{serviceShortName(service.name)}</span>
        <span className="text-xs font-bold text-ink-faint">{service.methods.length}</span>
      </summary>
      <ul className="mb-1 ml-3 border-l-2 border-line pl-1.5">
        <li>
          <Link
            to="/docs/api/$service"
            params={{ service: service.name }}
            activeOptions={{ exact: true }}
            className={cn(methodLink, "font-sans text-xs text-ink-soft")}
          >
            {service.name}
          </Link>
        </li>
        {service.methods.map((method) => (
          <li key={method.name}>
            <Link
              to="/docs/api/$service/$method"
              params={{ service: service.name, method: method.name }}
              className={methodLink}
            >
              {method.name}
            </Link>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** The column every reference page sits in. */
function Article({ children, wide }: { children: ReactNode; wide?: boolean }) {
  return (
    <div className={cn("mx-auto px-4 pt-6 pb-24", wide ? "max-w-6xl" : "max-w-3xl")}>
      {children}
    </div>
  );
}

function Heading({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="mt-9 mb-3 scroll-mt-4 text-xl font-black">
      {children}
    </h2>
  );
}

/** How a method can be called with a token. */
function Access({ method }: { method: dev.ApiMethod }) {
  const { t } = useTranslation("dev");
  if (method.access === dev.ApiAccess.SCOPE)
    return (
      <>
        {splitScopes(method.scope).map((scope) => (
          <Chip key={scope} className="bg-brand-soft font-mono text-brand-strong">
            {scope}
          </Chip>
        ))}
      </>
    );
  if (method.access === dev.ApiAccess.ANY_TOKEN)
    return <Chip className="bg-lime text-[#17122b]">{t("reference.access.anyToken")}</Chip>;
  return <Chip>{t("reference.access.sessionOnly")}</Chip>;
}

/** `/docs/api`: how the API works, then (on phones) the services. */
export function ReferenceHome() {
  const { t } = useTranslation("dev");
  const scopes = useQuery(scopesQuery);
  const eventTypes = useQuery(eventTypesQuery);
  const origin = window.location.origin;
  const quickStart = curlExample(
    methodUrl("/tokuly.dev.v1.TokenService/GetCurrentToken"),
    {},
    true,
  );

  return (
    <Article>
      <h1 className="text-3xl font-black">{t("reference.title")}</h1>
      <p className="mt-2 leading-relaxed text-ink-soft">{t("reference.lead")}</p>

      <div className="mt-5 flex flex-col gap-2">
        <p className="leading-relaxed">{t("reference.quickStart")}</p>
        <CodeBlock code={quickStart} language="shell" label="curl" />
      </div>

      <div className="mt-6 lg:hidden">
        <h2 className="mb-3 text-xl font-black">{t("reference.services")}</h2>
        <ServiceBrowser />
      </div>

      <Heading id="auth">{t("reference.auth.title")}</Heading>
      <Prose text={t("reference.auth.body")} />

      <Heading id="scopes">{t("reference.scopes.title")}</Heading>
      <Prose text={t("reference.scopes.body")} className="mb-3" />
      {scopes.isPending ? (
        <Skeleton className="h-64" />
      ) : scopes.isError ? (
        <Notice tone="danger">{errorMessage(scopes.error, { namespace: "dev" })}</Notice>
      ) : (
        <ul className="divide-y-2 divide-line rounded-2xl border-2 border-line">
          {scopes.data.map((scope) => (
            <li key={scope.scope} className="px-3 py-2">
              <p className="flex flex-wrap items-center gap-1.5">
                <code className="font-mono text-sm font-bold">{scope.scope}</code>
                {scope.sensitive && (
                  <Chip className="bg-sun text-[#17122b]">{t("scopes.sensitive")}</Chip>
                )}
              </p>
              <p className="text-sm leading-relaxed text-ink-soft">
                {localized(scope.description)}
              </p>
            </li>
          ))}
        </ul>
      )}

      <Heading id="errors">{t("reference.errors.title")}</Heading>
      <Prose text={t("reference.errors.body")} />

      <Heading id="oauth">{t("reference.oauth.title")}</Heading>
      <Prose text={t("reference.oauth.body")} className="mb-3" />
      <CodeBlock
        language="text"
        label={t("reference.oauth.endpoints")}
        code={[
          `GET  ${origin}/oauth/authorize`,
          `POST ${origin}${API_PATH}/oauth/token`,
          `POST ${origin}${API_PATH}/oauth/revoke`,
          `POST ${origin}${API_PATH}/oauth/device_authorization`,
          `GET  ${origin}${API_PATH}/.well-known/oauth-authorization-server`,
          `GET  ${origin}${API_PATH}/reference/descriptor.binpb`,
        ].join("\n")}
      />

      <Heading id="webhooks">{t("reference.webhooks.title")}</Heading>
      <Prose text={t("reference.webhooks.body")} className="mb-3" />
      <CodeBlock code={WEBHOOK_VERIFY_EXAMPLE} language="ts" label="verify.ts" />
      <h3 className="mt-5 mb-2 font-black">{t("reference.webhooks.events")}</h3>
      {eventTypes.isPending ? (
        <Skeleton className="h-48" />
      ) : eventTypes.isError ? (
        <Notice tone="danger">{errorMessage(eventTypes.error, { namespace: "dev" })}</Notice>
      ) : (
        <ul className="divide-y-2 divide-line rounded-2xl border-2 border-line">
          {eventTypes.data.map((event) => (
            <li key={event.type} className="px-3 py-2">
              <p className="flex flex-wrap items-center gap-1.5">
                <code className="font-mono text-sm font-bold">{event.type}</code>
                {splitScopes(event.scope).map((scope) => (
                  <Chip key={scope} className="font-mono">
                    {scope}
                  </Chip>
                ))}
              </p>
              <p className="text-sm leading-relaxed text-ink-soft">
                {localized(event.description)}
              </p>
            </li>
          ))}
        </ul>
      )}
    </Article>
  );
}

/** `/docs/api/<service>`: a service with its methods. */
export function ReferenceService({ service: name }: { service: string }) {
  const { t } = useTranslation("dev");
  const services = useServices();
  if (services.isPending)
    return (
      <Article>
        <Skeleton className="h-72" />
      </Article>
    );
  const service = services.data?.find((candidate) => candidate.name === name);
  if (!service)
    return (
      <Article>
        <EmptyState
          emoji="🧭"
          title={
            services.isError
              ? errorMessage(services.error, { namespace: "dev" })
              : t("reference.notFound")
          }
          action={<BackToStart />}
        />
      </Article>
    );
  return (
    <Article>
      <BackToStart />
      <p className="mt-3 font-mono text-xs font-black tracking-wider text-ink-faint uppercase">
        {service.domain}
      </p>
      <h1 className="text-2xl font-black break-words">{serviceShortName(service.name)}</h1>
      <p className="font-mono text-xs break-all text-ink-soft">{service.name}</p>
      <Prose text={service.description} className="mt-3" />
      <ul className="mt-5 divide-y-2 divide-line rounded-2xl border-2 border-line">
        {service.methods.map((method) => (
          <li key={method.name}>
            <Link
              to="/docs/api/$service/$method"
              params={{ service: service.name, method: method.name }}
              className="block px-3 py-2.5 hover:bg-surface-2"
            >
              <span className="flex flex-wrap items-center gap-1.5">
                <code className="font-mono text-sm font-bold">{method.name}</code>
                <Access method={method} />
              </span>
              <span className="mt-0.5 line-clamp-2 text-sm text-ink-soft">
                {method.description.replace(/\s+/g, " ")}
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </Article>
  );
}

function BackToStart() {
  const { t } = useTranslation("dev");
  return (
    <Link
      to="/docs/api"
      className="inline-flex items-center gap-1 text-sm font-bold text-ink-soft hover:text-ink"
    >
      <ChevronLeft className="size-4" />
      {t("reference.title")}
    </Link>
  );
}

/** `/docs/api/<service>/<method>`: one method: what it does, what it takes, examples, and a live call. */
export function ReferenceMethod({ service, method: name }: { service: string; method: string }) {
  const { t } = useTranslation("dev");
  const path = methodPath(service, name);
  const method = useMethod(path);
  const scopes = useQuery(scopesQuery);
  const [example, setExample] = useState<"curl" | "ts">("curl");

  if (method.isPending)
    return (
      <Article wide>
        <Skeleton className="h-96" />
      </Article>
    );
  if (method.isError || !method.data)
    return (
      <Article>
        <EmptyState
          emoji="🧭"
          title={errorMessage(method.error, { namespace: "dev" })}
          action={<BackToStart />}
        />
      </Article>
    );

  const data = method.data;
  const request = parseSchema(data.requestSchema);
  const response = parseSchema(data.responseSchema);
  const body = request ? exampleOf(request) : {};
  const streaming = data.clientStreaming || data.serverStreaming;
  const sessionOnly = data.access === dev.ApiAccess.SESSION_ONLY;
  const reasons = reasonsOf(data.description);
  const needed = splitScopes(data.scope);
  const url = methodUrl(data.path);

  return (
    <Article wide>
      <div className="xl:grid xl:grid-cols-[minmax(0,1fr)_26rem] xl:gap-8">
        <div className="min-w-0">
          <Link
            to="/docs/api/$service"
            params={{ service }}
            className="inline-flex max-w-full items-center gap-1 text-sm font-bold text-ink-soft hover:text-ink"
          >
            <ChevronLeft className="size-4 shrink-0" />
            <span className="truncate">{serviceShortName(service)}</span>
          </Link>
          <h1 className="mt-2 text-3xl font-black break-words">{data.name}</h1>
          <p className="mt-2 flex flex-wrap items-center gap-1.5">
            <Access method={data} />
            {streaming && <Chip>{t("reference.streaming")}</Chip>}
          </p>
          <div className="mt-3 flex items-center gap-2 rounded-2xl bg-surface-2 py-1.5 pr-1.5 pl-3.5">
            <span className="rounded-md bg-ink px-1.5 py-0.5 font-mono text-[11px] font-black text-bg">
              POST
            </span>
            <code className="min-w-0 flex-1 font-mono text-xs break-all">{data.path}</code>
            <CopyButton text={url} variant="ghost" className="h-8 shrink-0 px-2.5 text-xs" />
          </div>
          <Prose text={data.description} className="mt-4" />
          {sessionOnly && (
            <Notice className="mt-3">
              <Prose text={t("reference.access.sessionOnlyBody")} />
            </Notice>
          )}

          {needed.length > 0 && (
            <>
              <Heading id="scopes">{t("reference.method.scopes")}</Heading>
              <ul className="flex flex-col gap-1.5">
                {needed.map((scope) => (
                  <li key={scope} className="rounded-2xl bg-surface-2 px-3 py-2">
                    <code className="font-mono text-sm font-bold">{scope}</code>
                    <p className="text-sm leading-relaxed text-ink-soft">
                      {localized(scopes.data?.find((entry) => entry.scope === scope)?.description)}
                    </p>
                  </li>
                ))}
              </ul>
            </>
          )}

          <Heading id="request">{t("reference.method.request")}</Heading>
          <p className="mb-2 font-mono text-xs break-all text-ink-soft">{data.requestType}</p>
          {request && <SchemaView document={request} />}

          <Heading id="response">{t("reference.method.response")}</Heading>
          <p className="mb-2 font-mono text-xs break-all text-ink-soft">{data.responseType}</p>
          {response && <SchemaView document={response} />}

          {reasons.length > 0 && (
            <>
              <Heading id="errors">{t("reference.method.errors")}</Heading>
              <p className="mb-2 text-sm leading-relaxed text-ink-soft">
                {t("reference.method.errorsLead")}
              </p>
              <p className="flex flex-wrap gap-1.5">
                {reasons.map((reason) => (
                  <Chip key={reason} className="font-mono">
                    {reason}
                  </Chip>
                ))}
              </p>
            </>
          )}

          <Heading id="examples">{t("reference.method.examples")}</Heading>
          {streaming && <Notice className="mb-3">{t("reference.streamingBody")}</Notice>}
          <Tabs
            items={[
              { value: "curl" as const, label: "curl" },
              { value: "ts" as const, label: "TypeScript" },
            ]}
            value={example}
            onChange={(next: "curl" | "ts") => setExample(next)}
            className="mb-2"
          />
          {example === "curl" ? (
            <CodeBlock code={curlExample(url, body, !sessionOnly)} language="shell" label="curl" />
          ) : (
            <CodeBlock
              code={typescriptExample(url, body, !sessionOnly)}
              language="ts"
              label="TypeScript"
            />
          )}
        </div>

        <div className="mt-9 xl:mt-0">
          <div className="xl:sticky xl:top-6">
            {streaming ? (
              <Notice>{t("reference.tryIt.noStreaming")}</Notice>
            ) : (
              <TryIt key={data.path} path={data.path} initialBody={JSON.stringify(body, null, 2)} />
            )}
          </div>
        </div>
      </div>
    </Article>
  );
}
