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
  if (hash === "#faq" || hash === "#how-it-works") return "faq";
  if (hash === "#whitepaper") return "whitepaper";
  if (hash === "#protocols") return "protocols";
  return "main";
}
