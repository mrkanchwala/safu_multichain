import type { ReactNode } from "react";
import { DetailsLayout } from "./DetailsLayout";
import { FAQ_ANSWER, LINKS } from "../lib/links";
import { FactLink } from "../lib/facts";

const EXT = { target: "_blank", rel: "noopener noreferrer" } as const;

// Answers are always open (no accordion) so ctrl+F and search engines see all of them.
/** Every answer has a key, so a pool whose rules differ (the Solana test pool) can replace or hide it. */
export type FaqKey =
  | "what" | "losses" | "liquidation" | "decides" | "howStake" | "whichWallet" | "addCovered" | "withdraw"
  | "yield" | "amount" | "file" | "howLong" | "payoutWhere" | "afterPaid" | "safe" | "someoneElse" | "keys"
  | "fake" | "back" | "dapp" | "ask";
/** A pool's own answers: a node replaces the default, null hides the question. */
export type FaqOverrides = Partial<Record<FaqKey, ReactNode | null>>;

const GROUPS: { id: string; title: string; items: { key: FaqKey; q: string; a: ReactNode; id?: string }[] }[] = [
  {
    id: "faq-basics",
    title: "Basics",
    items: [
      {
        key: "what",
        q: "What is SAFU?",
        a: "SAFU is a shared pool that pays you back if crypto is stolen from a wallet you registered. You stake USDC into one pool on Stellar, choose up to three wallets to cover on Ethereum, Solana or Stellar, and if crypto is stolen from one of them, the pool pays you in USDC by rules that are fixed in advance.",
      },
      {
        key: "losses",
        q: "What kind of losses are covered?",
        a: "Two kinds: theft from your wallet (phishing, a malicious token approval, or a stolen private key or seed phrase) and wrongful liquidations of a loan held in one of your covered wallets. Covered assets are ETH, USDC and USDT on Ethereum, SOL, USDC and USDT on Solana, and XLM, USDC and USDT on Stellar.",
      },
      {
        key: "liquidation",
        q: "What is a wrongful liquidation?",
        a: "It's when a loan you hold on a lending market is liquidated at a price that wasn't real, for example because the market's price feed was pushed off for a moment. We check the price the lending market used against independent market price history, at the liquidation and again a while later. If it was far off both times, the market's price feed failed and the liquidation counts as a covered loss. A liquidation caused by a real price move isn't covered, and the exact window and threshold stay private so the check can't be gamed.",
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
        a: "Connect a Stellar, Ethereum or Solana wallet with the button at the top, open the Stake tab and enter an amount. If you stake from Ethereum or Solana, your USDC travels to the Stellar pool through Circle's CCTP, and SAFU pays the Stellar fees for that step. Each stake is currently 1 to 10 USDC, and the Stake tab shows the limits.",
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
        a: "After staking, register up to three wallets on any mix of the three chains. To show a wallet is yours, you send a tiny amount from that wallet to itself and we look for that transaction (on Stellar, a past transfer from it to your staking wallet also works). You never sign a message with the covered wallet. Those three wallets stay tied to your stake for good and can't be swapped later. Your staking wallet can't be one of its own covered wallets.",
      },
      {
        key: "withdraw",
        q: "Can I withdraw my stake?",
        a: "Yes, whenever there's no claim open or waiting on it. It goes back to the wallet you staked from, on its own chain, and your wallets stop being covered until you stake again.",
      },
      {
        key: "yield",
        q: "Is there any yield on my stake?",
        a: (
          <>
            Yes. The pool keeps some USDC on hand and puts the rest into a{" "}
            <FactLink fact="yieldVaultUrl">DeFindex vault</FactLink>{" "}
            that lends it on Blend's Fixed pool on Stellar. By default the yield on your own stake goes to you, and
            you'll see it in the Stake tab, added to what you can withdraw. Lending on Blend carries Blend's own
            smart-contract risk.
          </>
        ),
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
        a: "Open the File a claim tab within 30 days of the theft and paste the theft transaction. One claim can include up to 20 theft transactions across your three covered wallets, so if you were hit more than once, put them all in the same claim.",
      },
      {
        key: "howLong",
        q: "How long until I'm paid?",
        a: "If your stake is younger than 90 days, your claim is recorded at the full amount and held until the stake turns 90 days old. After that you approve the claim yourself, a 7-day cooldown runs, and the payout streams to you over 45 days, which you collect in the Collect payout tab.",
      },
      {
        key: "payoutWhere",
        q: "Where does the payout go?",
        a: "To the wallet you staked from, in USDC. If that wallet is on Ethereum or Solana, the last step is a receive transaction you approve in your own wallet, paying that chain's gas, and if you leave it unfinished you can complete it later.",
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
        a: "The contracts have their own test suites and fuzz testing, and every design choice we already know about is written up for the auditors. The Stellar contract gets an independent audit after launch. The report will be linked here. Your own keys never leave your wallet, and the pool only pays out against a signed, rule-checked result.",
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
        a: "A claim found to be fraudulent can be cancelled with those two keys, and the stake behind it is then locked for a year.",
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
        a: "Backers add USDC that helps the pool pay large claims on time, without registering any wallets of their own. Backed money matures after 7 days and can then be withdrawn, and backers always get back exactly what they put in, but they can only take out money that open claims don't need.",
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

export function Faq({ onBack, overrides = {} }: { onBack: () => void; overrides?: FaqOverrides }) {
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
          {g.items
            .filter((it) => overrides[it.key] !== null)
            .map((it) => (
              <div className="faq-item" key={it.key} id={it.id ? `faq-a-${it.id}` : undefined}>
                <h3>{it.q}</h3>
                <p>{overrides[it.key] ?? it.a}</p>
              </div>
            ))}
        </div>
      ))}
    </DetailsLayout>
  );
}
