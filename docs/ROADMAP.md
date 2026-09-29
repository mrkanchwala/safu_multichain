# Roadmap

## TL;DR

Tonight's build proves the mechanism: one USDC pool, three chains, two causes, deterministic payout. Everything after the event is about widening coverage and removing the trust assumptions a 36-hour build can't close on its own.

1. **On-chain covered-wallet registry.** Tonight's ownership fix lives off-chain, in the oracle's own table. Moving it on-chain, mirroring the pattern the pool already uses for a beneficiary's hashed address, removes "trust the oracle's bookkeeping" as an assumption.
2. **Close the two disclosed scanner gaps.** USDC/USDT loss amounts on Solana, and USDC loss amounts on Stellar itself. Both are parsing gaps against data the scanner already fetches, not new integrations.
3. **Cross-chain payout via CCTP.** Today, a loss on any chain settles in USDC on Stellar, correct, since the asset-matching rule means payout stays wherever the pool lives. A staker who wants their payout on the chain where the loss happened, rather than on Stellar, is the case CCTP would serve. Architecture only for this build; no code, by design (see "What this build deliberately does not include" below).
4. **Real price-at-loss-time conversion.** Tonight uses USDC-denominated test wallets to sidestep needing a price conversion for the demo. A real deployment needs to convert any lost asset to USDC at the price that held at the moment of loss, not the current price.
5. **Independent security review**, then mainnet, for the yield-split and cross-chain claim logic added this build, on top of the review the base pool already carries.
6. **More chains, same shape.** The chain list is a config table (allowlist entries, an RPC URL, a chain-id registration), not new contract logic. Extending it costs backend work, not a new pool.

## Immediately after the event

**On-chain covered-wallet registry.** The single biggest trust-assumption this build carries. Moving it on-chain closes the gap between "the oracle says this wallet was registered in time" and "the chain itself can prove it."

**Close the Solana and Stellar scanner gaps.** Both chains' scanners already fetch the data that's missing an amount, a Solana SPL balance-delta path, and a Stellar trustline amount parsed as a number instead of left as a string. Neither needs a new integration.

**Independent security review.** The base protection-pool this build forks already carries a security review as part of SAFU's mainnet deployment. What's new here, the yield-split accounting, the cross-chain claim wrapper, the wrongful-liquidation detector, has not been reviewed independently and should be, before any of it touches a chain where the funds are real.

## Next two quarters

**Cross-chain payout via CCTP.** Circle's CCTP already moves USDC natively between the chains this build covers. The architecture for this is straightforward given the asset-matching rule already in place: a payout destined for a chain other than Stellar routes through CCTP instead of settling locally. This is deliberately not built for the event, see the scope note below.

**Real loss-to-USDC pricing.** Reflector already carries live prices for every asset this build covers; the missing piece is capturing the price at the moment of *detection* (not at claim time, since Reflector's retention window is far shorter than the claim-filing window) and using it for assets other than the ones already priced in USDC terms.

**More lending markets, more chains.** The wrongful-liquidation detector is protocol-agnostic by construction, it takes a price, a timestamp, and an asset, not a specific lending market's types. Covering a new market or a new chain is an integration, not a rebuild of the detector.

**LayerZero as the on-chain covered-wallet registry's messaging layer. Built and proven on testnet for this event; mainnet needs a real verifier.** Raised directly by a member of the Stellar team as a strategic direction. The pool stays USDC-denominated, LayerZero is not a USDC bridge here (that job is CCTP, above); LayerZero is a generic cross-chain messaging protocol, and its natural fit is the *first* item on this roadmap: relaying a real covered-wallet registration event on Ethereum or Solana to Stellar in a verifiable way, closing the "trust the oracle's off-chain table" gap with an on-chain mechanism instead.

*What exists and was run (2026-09-19):* a Soroban `registry-oapp` on Stellar testnet, a Sepolia sender and a Solana devnet sender. A real message from each chain was sent through LayerZero's real endpoints and delivered into the Stellar registry; each commitment matches what the off-chain registry computed beforehand.

*Stated plainly:* the verifier on those runs was **one we operate ourselves on testnet**, because LayerZero's default verifier did not verify these testnet routes in the time we watched. The messages still travel through LayerZero's endpoints and message library, but the independent attestation that LayerZero's own verifier network would give is replaced by our signature. This is a testnet demonstration, not a trust claim. For mainnet the registry would require LayerZero's production verifiers (ideally more than one), and the off-chain registry remains the primary gate for claims either way, since a late or missing relay must never block a genuine claim.

## What this build deliberately does not include

**Cross-chain message-passing code, of any kind.** Not built, not stubbed, not partially wired, a founder decision held through the whole build, restated explicitly here because it is easy to reach for "just add a bridge" once multichain coverage exists. The current design doesn't need one: the asset-matching rule already means a payout settles wherever the pool lives, and CCTP (above) is the correct tool for the one case that does need cross-chain movement, moving USDC itself, not passing an arbitrary message.

**Detection thresholds or scoring internals.** The wrongful-liquidation price-deviation check and the drain scanner's own scoring are described in the design doc by mechanism, not by exact number, for the same reason SAFU never publishes its fraud-scoring internals, publishing the number makes the check easier to design around, not harder to trust.

## Funding intent

SAFU holds an active Stellar Community Fund award and is mid-delivery on its third tranche, with a mainnet protection pool already live. The roadmap above extends that same pool's coverage rather than starting a new track, a follow-on application would cover the independent review, the on-chain registry, and the cross-chain payout work, none of which fits inside a 36-hour build by design.
