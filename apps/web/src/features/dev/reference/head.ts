const SITE = "Tokuly API";
const DESCRIPTION =
  "The Tokuly API: every service and method with request and response shapes, scopes, errors, examples and a live try-it panel.";

/**
 * The `<head>` of a reference page. The pages render in the browser, so the
 * title and description come from the address alone and are in the HTML the
 * server sends: link previews and search results name the right method.
 */
export function referenceHead(page?: string, { description = DESCRIPTION } = {}) {
  const title = page ? `${page} · ${SITE}` : SITE;
  return {
    meta: [
      { title },
      { name: "description", content: description },
      { property: "og:title", content: title },
      { property: "og:description", content: description },
      { property: "og:type", content: "website" },
    ],
  };
}
