import type { FaqOverrides } from "../../shell/components/Faq";
import { num, setting } from "./lib/pool";
import { fmtDuration } from "./lib/reads";
import { usePool } from "./lib/usePool";

// FAQ answers for the Solana test pool (B42, founder 16 Nov): only where its rules differ from the default
// answers. Every number is read from the program's IDL or the pool's live settings, never typed here.

const wallets = () => num("MAX_COVERED_WALLETS").toString();
const days = (name: string) => fmtDuration(Number(num(name)));

/** A live pool setting as a duration ("4 minutes"); "a short" while the pool is still loading. */
function LiveDuration({ name }: { name: string }) {
  const { pool } = usePool();
  return <>{pool ? fmtDuration(Number(setting(pool, name))) : "a short"}</>;
}

export const SOLANA_FAQ: FaqOverrides = {
  what: (
    <>
      This is SAFU's Solana pool, running on Solana devnet: a test network, so no real money is involved. You stake
      SOL, choose up to {wallets()} wallets to cover on Solana or Ethereum, and if crypto is stolen from one of them,
      the pool pays you in SOL by rules fixed in advance.
    </>
  ),
  losses: "Theft from a covered wallet: phishing, a malicious token approval, or a stolen private key or seed phrase.",
  liquidation: null,
  howStake:
    "Connect a Solana wallet switched to devnet with the button at the top, open the Stake tab and enter an amount in SOL. The Stake tab shows the current limits.",
  addCovered: (
    <>
      After staking, register up to {wallets()} wallets, Solana or Ethereum in any mix. To show a wallet is yours,
      you send a tiny amount from that wallet to itself and we look for that transaction. You never sign a message
      with the covered wallet. Your staking wallet can't be one of its own covered wallets.
    </>
  ),
  yield:
    "Yes. Most of the pool's SOL is staked through Marinade, and your share of that yield shows in the Stake tab. Taking SOL back out of Marinade costs Marinade's unstake fee, and the Stake tab shows it before you withdraw.",
  file: (
    <>
      Open the File a claim tab within {days("CLAIM_WINDOW_SECS")} of the theft and paste the theft transaction. One
      claim can include up to {num("MAX_TXIDS_PER_CLAIM").toString()} theft transactions across your {wallets()}{" "}
      covered wallets.
    </>
  ),
  howLong: (
    <>
      This test pool runs on shortened clocks. A claim on a stake younger than {days("TIME_GATE_SECS")} is recorded
      at the full amount and held until then. After that you approve the claim yourself, a{" "}
      <LiveDuration name="COOLDOWN_SECS" /> cooldown runs, and the payout streams to you over{" "}
      <LiveDuration name="VESTING_SECS" />, which you collect in the Collect payout tab.
    </>
  ),
  payoutWhere: "To the wallet you staked from, in SOL.",
  safe:
    "This pool runs on Solana devnet, a test network, so no real money is at risk. The program has its own test suites and fuzz testing. Your keys never leave your wallet, and the pool only pays out against a signed, rule-checked result.",
  fake: (
    <>
      A claim found to be fraudulent can be cancelled with those two keys, and the stake behind it is then locked for{" "}
      {days("PENALTY_LOCK_SECS")} on this test pool.
    </>
  ),
  back: (
    <>
      Backers add SOL that helps the pool pay large claims on time, without registering any wallets of their own.
      Backed SOL matures after <LiveDuration name="BACKER_MATURITY_SECS" /> on this test pool and can then be
      withdrawn, as long as open claims don't need it.
    </>
  ),
};
