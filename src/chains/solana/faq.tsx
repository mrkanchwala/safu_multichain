import type { PoolFaq } from "../../shell/components/Faq";
import { num, setting } from "./lib/pool";
import { fmtDuration } from "./lib/reads";
import { usePool } from "./lib/usePool";

// The Solana test pool's FAQ answers (B42, founder 16 Nov; CIE M1): every question whose answer depends on
// this pool's rules. Every number is read from the program's IDL or the pool's live settings, never typed here.

const wallets = () => num("MAX_COVERED_WALLETS").toString();
const days = (name: string) => fmtDuration(Number(num(name)));
/** A tier's ceiling as a multiple of the stake, as the program computes it (pool_core::claim::tier_cap). */
const ceiling = (tier: "A" | "B" | "C") =>
  (Number(num(`TIER_${tier}_RATIO`)) * Number(num("TIER_COVERAGE_BPS"))) / Number(num("BPS_DENOMINATOR"));

/** A live pool setting as a duration ("4 minutes"); "a short" while the pool is still loading. */
function LiveDuration({ name }: { name: string }) {
  const { pool } = usePool();
  return <>{pool ? fmtDuration(Number(setting(pool, name))) : "a short"}</>;
}

export const SOLANA_FAQ: PoolFaq = {
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
  withdraw:
    "Yes, whenever there's no claim open on it. It goes back to the wallet you staked from, in SOL, and your wallets stop being covered until you stake again.",
  yield:
    "Yes. Most of the pool's SOL is staked through Marinade, and your share of that yield shows in the Stake tab. Taking SOL back out of Marinade costs Marinade's unstake fee, and the Stake tab shows it before you withdraw.",
  amount: (
    <>
      Up to {ceiling("A")} times your stake for a Tier A wallet, {ceiling("B")} times for Tier B and {ceiling("C")}{" "}
      times for Tier C, and never more than you actually lost. Each covered wallet's tier comes from its own history,
      so older, steadier wallets rank higher. These are ceilings: the payout is the smaller of the ceiling and your
      real loss.
    </>
  ),
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
  afterPaid:
    "One stake covers one claim, so it's used up, even if the payout came in under the ceiling. A staking wallet that has had a claim paid can't stake again.",
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
