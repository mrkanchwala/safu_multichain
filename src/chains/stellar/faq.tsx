import type { PoolFaq } from "../../shell/components/Faq";
import { FactLink } from "../../shell/lib/facts";

// The Stellar pool's FAQ answers (CIE M1, B42): every question whose answer depends on this pool's rules.
// Moved as-is from shell/components/Faq.tsx; the shell keeps only answers that hold for every pool.

export const STELLAR_FAQ: PoolFaq = {
  what: "SAFU is a shared pool that pays you back if crypto is stolen from a wallet you registered. You stake USDC into one pool on Stellar, choose up to three wallets to cover on Ethereum, Solana or Stellar, and if crypto is stolen from one of them, the pool pays you in USDC by rules that are fixed in advance.",
  losses:
    "Two kinds: theft from your wallet (phishing, a malicious token approval, or a stolen private key or seed phrase) and wrongful liquidations of a loan held in one of your covered wallets. Covered assets are ETH, USDC and USDT on Ethereum, SOL, USDC and USDT on Solana, and XLM, USDC and USDT on Stellar.",
  liquidation:
    "It's when a loan you hold on a lending market is liquidated at a price that wasn't real, for example because the market's price feed was pushed off for a moment. We check the price the lending market used against independent market price history, at the liquidation and again a while later. If it was far off both times, the market's price feed failed and the liquidation counts as a covered loss. A liquidation caused by a real price move isn't covered, and the exact window and threshold stay private so the check can't be gamed.",
  howStake:
    "Connect a Stellar, Ethereum or Solana wallet with the button at the top, open the Stake tab and enter an amount. If you stake from Ethereum or Solana, your USDC travels to the Stellar pool through Circle's CCTP, and SAFU pays the Stellar fees for that step. Each stake is currently 1 to 10 USDC, and the Stake tab shows the limits.",
  addCovered:
    "After staking, register up to three wallets on any mix of the three chains. To show a wallet is yours, you send a tiny amount from that wallet to itself and we look for that transaction (on Stellar, a past transfer from it to your staking wallet also works). You never sign a message with the covered wallet. Those three wallets stay tied to your stake for good and can't be swapped later. Your staking wallet can't be one of its own covered wallets.",
  withdraw:
    "Yes, whenever there's no claim open or waiting on it. It goes back to the wallet you staked from, on its own chain, and your wallets stop being covered until you stake again.",
  yield: (
    <>
      Yes. The pool keeps some USDC on hand and puts the rest into a{" "}
      <FactLink fact="yieldVaultUrl">DeFindex vault</FactLink> that lends it on
      Blend's Fixed pool on Stellar. By default the yield on your own stake goes
      to you, and you'll see it in the Stake tab, added to what you can
      withdraw. Lending on Blend carries Blend's own smart-contract risk.
    </>
  ),
  amount:
    "Up to 15 times your stake for a Tier A wallet, 10 times for Tier B and 5 times for Tier C, and never more than you actually lost. Each covered wallet's tier comes from its own history, so older, steadier wallets rank higher. These are ceilings: the payout is the smaller of the ceiling and your real loss.",
  file: "Open the File a claim tab within 30 days of the theft and paste the theft transaction. One claim can include up to 20 theft transactions across your three covered wallets, so if you were hit more than once, put them all in the same claim.",
  howLong:
    "If your stake is younger than 90 days, your claim is recorded at the full amount and held until the stake turns 90 days old. After that you approve the claim yourself, a 7-day cooldown runs, and the payout streams to you over 45 days, which you collect in the Collect payout tab.",
  payoutWhere:
    "To the wallet you staked from, in USDC. If that wallet is on Ethereum or Solana, the last step is a receive transaction you approve in your own wallet, paying that chain's gas, and if you leave it unfinished you can complete it later.",
  afterPaid:
    "One stake covers one claim, so it's used up, even if the payout came in under the ceiling. A staking wallet that has had a claim paid can't stake again.",
  safe: "The contracts have their own test suites and fuzz testing, and every design choice we already know about is written up for the auditors. The Stellar contract gets an independent audit after launch. The report will be linked here. Your own keys never leave your wallet, and the pool only pays out against a signed, rule-checked result.",
  fake: "A claim found to be fraudulent can be cancelled with those two keys, and the stake behind it is then locked for a year.",
  back: "Backers add USDC that helps the pool pay large claims on time, without registering any wallets of their own. Backed money matures after 7 days and can then be withdrawn, and backers always get back exactly what they put in, but they can only take out money that open claims don't need.",
};
