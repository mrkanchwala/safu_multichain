import type { ReactNode } from "react";
import { DetailsLayout } from "./DetailsLayout";
import { FAQ_ANSWER, LINKS } from "../lib/links";

const EXT = { target: "_blank", rel: "noopener noreferrer" } as const;

// Answers are always open (no accordion) so ctrl+F and search engines see all of them.
// One FAQ for every pool and chain (founder 16 Nov): no chain, asset or vendor names, no pool numbers except the
// 15x ceiling. Each pool's own limits show in its app.
type Item = { key: string; q: string; a: ReactNode; id?: string };

const GROUPS: { id: string; title: string; items: Item[] }[] = [
  {
    id: "faq-basics",
    title: "Basics",
    items: [
      {
        key: "what",
        q: "What is SAFU?",
        a: "SAFU is a shared pool that pays you back if crypto is stolen from a wallet you registered. You stake into a pool, choose the wallets you want covered, and if crypto is stolen from one of them, the pool pays you automatically, by rules fixed in advance.",
      },
      {
        key: "losses",
        q: "What kind of losses are covered?",
        a: "Theft from a covered wallet: phishing, a malicious token approval, or a stolen private key or seed phrase. Some pools also cover a wrongful liquidation of a loan held in a covered wallet. Each pool lists the chains and assets it covers in its app.",
      },
      {
        key: "liquidation",
        q: "What is a wrongful liquidation?",
        a: "It's when a loan you hold on a lending market is liquidated at a price that wasn't real, for example because the market's price feed was pushed off for a moment. We check the price the lending market used against independent market price history. If it was far off, the liquidation counts as a covered loss. A liquidation caused by a real price move isn't covered, and the exact window and threshold stay private so the check can't be gamed.",
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
        a: "Connect a wallet with the button at the top, open the Stake tab and enter an amount. The Stake tab shows the pool's current limits.",
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
        a: "After staking, register the wallets you want covered. To show a wallet is yours, you send a tiny amount from that wallet to itself and we look for that transaction. You never sign a message with the covered wallet. Your staking wallet can't be one of its own covered wallets.",
      },
      {
        key: "withdraw",
        q: "Can I withdraw my stake?",
        a: "Yes, whenever there's no claim open or waiting on it. It goes back to the wallet you staked from, and your wallets stop being covered until you stake again.",
      },
      {
        key: "yield",
        q: "Is there any yield on my stake?",
        a: "Yes. The pool keeps some funds on hand and can lend the rest on an established lending market. By default the yield on your own stake goes to you, and you'll see it in the Stake tab. Lending carries that market's own smart-contract risk.",
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
        a: "Up to 15 times your stake for a Tier A wallet, 10 times for Tier B and 5 times for Tier C, and never more than you actually lost. Each covered wallet's tier comes from its own history, so older, steadier wallets rank higher. These are ceilings: the payout is the smaller of the ceiling and your real loss.",
      },
      {
        key: "file",
        q: "How do I file a claim?",
        a: "Open the File a claim tab within the pool's claim window and paste the theft transaction. If you were hit more than once, put every theft transaction in the same claim.",
      },
      {
        key: "howLong",
        q: "How long until I'm paid?",
        a: "A claim on a new stake is recorded at its full amount and held until the stake has been in the pool long enough. After that you approve the claim yourself, a short cooldown runs, and the payout streams to you, which you collect in the Collect payout tab. Payouts are automatic: nobody votes on them.",
      },
      {
        key: "payoutWhere",
        q: "Where does the payout go?",
        a: "To the wallet you staked from. If that wallet is on a different chain from the pool, the last step is a receive transaction you approve in your own wallet.",
      },
      {
        key: "afterPaid",
        q: "What happens to my stake after a claim is paid?",
        a: "One stake covers one claim, so it's used up, even if the payout came in under the ceiling. A staking wallet that has had a claim paid can't stake again.",
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
        a: "Each pool contract has its own test suites and fuzz testing, and the design choices we already know about are written up for the auditors. Audit reports will be linked here. Your keys never leave your wallet, and a pool only pays out against a signed, rule-checked result.",
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
        a: "A claim found to be fraudulent can be cancelled with those two keys, and the stake behind it is then locked.",
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
        a: "Backers add funds that help the pool pay large claims on time, without registering any wallets of their own. Backing matures after a short period and can then be withdrawn, and backers always get back exactly what they put in, but they can only take out money that open claims don't need.",
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

export function Faq({ onBack }: { onBack: () => void }) {
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
            return (
              <div className="faq-item" key={it.key} id={it.id ? `faq-a-${it.id}` : undefined}>
                <h3>{it.q}</h3>
                <p>{it.a}</p>
              </div>
            );
          })}
        </div>
      ))}
    </DetailsLayout>
  );
}
