/** Example calls for a method, as text to show and copy. */

const pretty = (body: unknown) => JSON.stringify(body, null, 2);

/** `text` indented by `spaces`, except its first line. */
const indentRest = (text: string, spaces: number) =>
  text.replaceAll("\n", `\n${" ".repeat(spaces)}`);

/** A `curl` call of the method. The token comes from `$TOKULY_TOKEN`, never from the page. */
export function curlExample(url: string, body: unknown, authenticated: boolean): string {
  // A single quote cannot be escaped inside single quotes: close, escape, reopen.
  const data = pretty(body).replaceAll("'", "'\\''");
  return [
    `curl ${url} \\`,
    ...(authenticated ? ['  -H "Authorization: Bearer $TOKULY_TOKEN" \\'] : []),
    '  -H "Content-Type: application/json" \\',
    `  -d '${data}'`,
  ].join("\n");
}

/** The same call with `fetch`, reading the error reason from its header. */
export function typescriptExample(url: string, body: unknown, authenticated: boolean): string {
  return [
    `const response = await fetch("${url}", {`,
    '  method: "POST",',
    "  headers: {",
    // biome-ignore lint/suspicious/noTemplateCurlyInString: the example is code, shown as text.
    ...(authenticated ? ["    Authorization: `Bearer ${process.env.TOKULY_TOKEN}`,"] : []),
    '    "Content-Type": "application/json",',
    "  },",
    `  body: JSON.stringify(${indentRest(pretty(body), 2)}),`,
    "});",
    "if (!response.ok) {",
    "  // A stable code such as INSUFFICIENT_SCOPE; the body has the details.",
    '  throw new Error(response.headers.get("x-tokuly-error") ?? response.statusText);',
    "}",
    "const data = await response.json();",
  ].join("\n");
}

/** Checking the signature of a webhook delivery (Node.js). */
export const WEBHOOK_VERIFY_EXAMPLE = `import { createHmac, timingSafeEqual } from "node:crypto";

// Tokuly-Signature: t=<unix seconds>,v1=<hex>[,v1=<hex>]
export function verify(secret: string, header: string, body: string): boolean {
  const parts = header.split(",").map((part) => part.split("="));
  const time = parts.find(([key]) => key === "t")?.[1] ?? "";
  // Refuse deliveries that are more than five minutes old.
  if (Math.abs(Date.now() / 1000 - Number(time)) > 300) return false;
  const expected = createHmac("sha256", secret).update(\`\${time}.\${body}\`).digest();
  return parts
    .filter(([key]) => key === "v1")
    .some(([, hex = ""]) => {
      const given = Buffer.from(hex, "hex");
      return given.length === expected.length && timingSafeEqual(given, expected);
    });
}`;
