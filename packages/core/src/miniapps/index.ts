/**
 * Mini-app logic every host app shares (`docs/miniapps.md`): query keys and
 * options, the address of a mini app inside Tokuly and of a card that opens
 * it, what a bridge call needs before it may run, and a person as a mini app
 * sees them. The frame, the bridge's transport and the sheets stay in each app.
 */

export {
  cardMessage,
  type MiniAppLink,
  miniAppHref,
  nextStep,
  personOf,
  readMiniAppLink,
  type Step,
} from "./model";
export { miniAppKeys, miniAppQueries } from "./queries";
