# registry-oapp

Covered-wallet registry delivered over LayerZero. A registration made on Ethereum or Solana is relayed
to Stellar and recorded as `sha256(staker_hash(32) ++ chain_id(1) ++ wallet_hash(32))`.
`chain_id`: 1 = Ethereum (incl. Sepolia), 2 = Solana.

Standalone crate. It has its own storage and never touches `protection-pool`.

## Role in SAFU

The off-chain registry (`backend/registry.py`) is the PRIMARY gate for claims. The on-chain commitment
adds a verification label (`on-chain-verified` / `off-chain-only`). It never blocks a claim: a late or
missing relay must not lose a genuine claim.

## Trust model, stated plainly

- **Message verification is LayerZero's, not ours.** This contract calls the endpoint's `clear` before
  it stores anything, and stores nothing if `clear` rejects (unit-tested against a rejecting endpoint).
  Whether the real endpoint rejects an unverified message is only proven by the live testnet e2e.
- **The sender is pinned per source chain** via `set_peer`. A wrong or unconfigured sender is rejected
  (`OnlyPeer` / `NoPeer`), before our code runs.
- **Replay is the endpoint's job.** A re-delivery here is also a no-op that keeps the first timestamp.
- **Both testnet pathways (Stellar to Solana, Stellar to Sepolia) use the same single DVN operator**
  (read live on 2026-09-19). Verification is therefore one party's attestation, not a multi-party
  guarantee. Do not describe this as trustless.
- **The owner key is the trust root.** It sets peers and grants roles. Those functions come from
  LayerZero's framework and are auth-gated (unauthorized `set_peer` and `grant_role` are rejected in
  tests), but they were not written by us and have had no independent audit.
- **No upgrade path.** The built wasm exposes no upgrade function.
- **Entries expire.** A registration's storage TTL is bumped to about 120 days on write.
  `extend_registration_ttl(commitment)` is permissionless and refreshes it. After expiry `is_registered`
  returns `None`, which degrades to the off-chain gate, not to a rejected claim.

## Scope of review

Basic self-review against the Soroban vulnerability checklist plus unit tests. This is NOT a full audit.
