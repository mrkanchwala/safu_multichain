import { SITE } from "./site";

// External links shared by the header, footer and the explainer pages (values: hq/site.toml).
export const LINKS = {
  site: SITE.site.url,
  feedback: SITE.links.feedback,
  x: SITE.links.x,
  telegram: SITE.links.telegram,
  github: SITE.links.github,
  bdIntake: SITE.links.bd_intake,
} as const;

// Hash routes: the FAQ and the whitepaper get their own URL so they can be shared.
// Anything else (including "#about") renders the main page. "#how-it-works" is the
// FAQ's old address, kept so earlier links still land on it.
export type View = "main" | "faq" | "whitepaper" | "protocols" | "liquidity";

// The Liquidity tab (F20): "#liquidity", or "#liquidity/<settings>" for a shared setup.
export const LIQUIDITY_HASH = "#liquidity";
export const LIQUIDITY_STATE_PREFIX = `${LIQUIDITY_HASH}/`;

export function viewFromHash(hash: string): View {
  if (hash === "#faq" || hash === "#how-it-works" || hash.startsWith("#faq/")) return "faq";
  if (hash === "#whitepaper") return "whitepaper";
  if (hash === "#protocols") return "protocols";
  if (hash === LIQUIDITY_HASH || hash.startsWith(LIQUIDITY_STATE_PREFIX)) return "liquidity";
  return "main";
}

// One FAQ answer, linked from the stake panel (B19): "#faq/<id>" opens the FAQ at that answer.
export const FAQ_ANSWER = { whichWallet: "which-wallet", addCovered: "add-covered" } as const;

export function faqAnswerHref(id: string): string {
  return `#faq/${id}`;
}

/** Scroll to the answer a "#faq/<id>" hash names, if any. True when it found one. */
export function scrollToFaqAnswer(hash: string): boolean {
  if (!hash.startsWith("#faq/")) return false;
  const el = document.getElementById(`faq-a-${hash.slice(5)}`);
  el?.scrollIntoView();
  return el !== null;
}
