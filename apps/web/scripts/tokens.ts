// Writes src/tokens.css from @tokuly/core/tokens (Node runs this TypeScript as it is).
import { writeFileSync } from "node:fs";
import { tokensCss } from "../src/lib/tokens-css.ts";

writeFileSync(new URL("../src/tokens.css", import.meta.url), tokensCss());
