import type { ReactNode } from "react";
import { DetailsLayout } from "./DetailsLayout";
import { FAQ_ANSWER, LINKS } from "../lib/links";

const EXT = { target: "_blank", rel: "noopener noreferrer" } as const;

// Answers are always open (no accordion) so ctrl+F and search engines see all of them.
/** Questions whose answer depends on the pool's own rules. Each pool answers every one of them in its adapter
 *  (chains/<pool>/faq.tsx, CIE M1, B42); null = the question doesn't apply to that pool and is hidden. */
export type PoolFaqKey =
  | "what" | "losses" | "liquidation" | "howStake" | "addCovered" | "withdraw" | "yield" | "amount" | "file"
  | "howLong" | "payoutWhere" | "afterPaid" | "safe" | "fake" | "back";
export type PoolFaq = Record<PoolFaqKey, ReactNode | null>;
/** Questions with one answer for every pool, kept here. */
type SharedFaqKey = "decides" | "whichWallet" | "someoneElse" | "keys" | "dapp" | "ask";

type Item = { key: PoolFaqKey; q: string; id?: string } | { key: SharedFaqKey; q: string; a: ReactNode; id?: string };

const GROUPS: { id: string; title: string; items: Item[] }[] = [
  {
    id: "faq-basics",
    title: "Basics",
    items: [
      {
        key: "what",
        q: "What is SAFU?",
      },
      {
        key: "losses",
        q: "What kind of losses are covered?",
      },
      {
        key: "liquidation",
        q: "What is a wrongful liquidation?",
      },
      {
        key: "decides",
        q: "Who decides whether I get paid?",
        a: "The rules decide: our scanner checks the transaction on the chain it happened on, the oracle signs the result, and the pool contract checks that signature against its own limits before anything moves. The same transaction gets the same answer every time, and nobody votes on it.",
      },
    ],
  },
  {
    id: "faq-staking",
    title: "Staking",
    items: [
      {
        key: "howStake",
        q: "How do I stake?",
      },
      {
        id: FAQ_ANSWER.whichWallet,
        key: "whichWallet",
        q: "Which wallet should I stake from?",
        a: "A fresh wallet that you keep safe, because payouts and withdrawals go back to the wallet you staked from. The wallets you want protected are registered separately as covered wallets, and you never connect those to this site.",
      },
      {
        id: FAQ_ANSWER.addCovered,
        key: "addCovered",
        q: "How do I add the wallets I want covered?",
      },
      {
        key: "withdraw",
        q: "Can I withdraw my stake?",
      },
      {
        key: "yield",
        q: "Is there any yield on my stake?",
      },
    ],
  },
  {
    id: "faq-claims",
    title: "Claims and payouts",
    items: [
      {
        key: "amount",
        q: "How much can I get paid?",
      },
      {
        key: "file",
        q: "How do I file a claim?",
      },
      {
        key: "howLong",
        q: "How long until I'm paid?",
      },
      {
        key: "payoutWhere",
        q: "Where does the payout go?",
      },
      {
        key: "afterPaid",
        q: "What happens to my stake after a claim is paid?",
      },
    ],
  },
  {
    id: "faq-security",
    title: "Security",
    items: [
      {
        key: "safe",
        q: "Is SAFU safe to use?",
      },
      {
        key: "someoneElse",
        q: "Can someone else claim for my wallet?",
        a: "No, because a claim can only name a wallet that was registered to your stake before the theft happened, so citing someone else's public theft afterwards gets nothing.",
      },
      {
        key: "keys",
        q: "Who holds the keys?",
        a: "You hold your own wallets' keys, and SAFU never asks for them. The oracle key that signs claim results sits in a cloud key-management service and never leaves it, and a few admin actions, such as cancelling a claim found to be fraudulent, need two separate keys to sign.",
      },
      {
        key: "fake",
        q: "What happens if someone files a fake claim?",
      },
    ],
  },
  {
    id: "faq-other",
    title: "Other",
    items: [
      {
        key: "back",
        q: 'What does "Back the pool" mean?',
      },
      {
        key: "dapp",
        q: "I run a consumer dApp or platform. Can we use SAFU?",
        a: (
          <>
            Yes, you can run a SAFU pool under your own name for your own users. See{" "}
            <a href="#protocols">For protocols</a>
            .
          </>
        ),
      },
      {
        key: "ask",
        q: "Where can I ask something else?",
        a: (
          <>
            Message us on{" "}
            <a href={LINKS.telegram} {...EXT}>
              Telegram
            </a>{" "}
            or{" "}
            <a href={LINKS.x} {...EXT}>
              X
            </a>
            , or send us a note through the{" "}
            <a href={LINKS.feedback} {...EXT}>
              feedback form
            </a>
            .
          </>
        ),
      },
    ],
  },
];

export function Faq({ onBack, answers }: { onBack: () => void; answers: PoolFaq }) {
  return (
    <DetailsLayout
      sections={GROUPS.map((g) => ({ id: g.id, label: g.title }))}
      cross={{ label: "Read the whitepaper →", href: "#whitepaper" }}
      onBack={onBack}
    >
      <div className="wp-head">
        <h1>Questions</h1>
        <div className="sub-line">Staking, claims, payouts and security, in plain terms.</div>
      </div>
      {GROUPS.map((g) => (
        <div className="slide faq-group" id={g.id} key={g.id}>
          <h2>{g.title}</h2>
          {g.items.map((it) => {
            const a = "a" in it ? it.a : answers[it.key];
            return a === null ? null : (
              <div className="faq-item" key={it.key} id={it.id ? `faq-a-${it.id}` : undefined}>
                <h3>{it.q}</h3>
                <p>{a}</p>
              </div>
            );
          })}
        </div>
      ))}
    </DetailsLayout>
  );
}
