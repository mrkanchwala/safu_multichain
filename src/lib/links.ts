// External links shared by the header, footer and the explainer pages, the same
// targets as the main safustaking.com site.
export const LINKS = {
  site: "https://safustaking.com",
  feedback: "https://docs.google.com/forms/d/1qD9IrIkfs39Wupaw5y-zvCS46Ppq4JQiIsRgV9D3s80/viewform",
  x: "https://x.com/safu_staking",
  telegram: "https://t.me/+sza3oozmzzJlNzk0",
    github: "https://github.com/mrkanchwala",
  bdIntake: "https://docs.google.com/forms/d/e/1FAIpQLSefYrJjgxzJrHJe59U7BFottI7KrQMwgiq_txmShbxBBmWk9w/viewform",
} as const;

// Hash routes: the FAQ and the whitepaper get their own URL so they can be shared.
// Anything else (including "#about") renders the main page. "#how-it-works" is the
// FAQ's old address, kept so earlier links still land on it.
export type View = "main" | "faq" | "whitepaper" | "protocols";

export function viewFromHash(hash: string): View {
  if (hash === "#faq" || hash === "#how-it-works" || hash.startsWith("#faq/")) return "faq";
  if (hash === "#whitepaper") return "whitepaper";
  if (hash === "#protocols") return "protocols";
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
