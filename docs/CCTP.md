# CCTP: staking and backing from Ethereum and Solana

Users on Ethereum or Solana stake into (or back) the Stellar pool with USDC
over Circle CCTP v2. The stake lives on Stellar; the user never holds a
Stellar key.

Contracts: `contracts/cctp-adapter`, `contracts/safu-account`,
`contracts/cctp-common`. Live testnet round trip proven 2026-09-22
(Sepolia burn → stake on Stellar → EVM-signed withdrawal → USDC back on
Sepolia).

## How it works

1. The user burns USDC on their home chain with **a SAFU adapter as both
   `mintRecipient` and `destinationCaller`**.
2. Circle attests the burn. A relayer calls `mint_and_stake(message,
   attestation)` on the adapter.
3. The adapter creates the user's own account on first use (fixed address,
   `account_address(home_domain, owner)`), moves the USDC in, and the account
   stakes it, or backs the pool, depending on the adapter.
4. The user controls the account with their home-chain wallet signature. Money
   leaves only through `*_home` functions, back to their home address.

**Two adapters, one per purpose** (`DepositMode`): a staking adapter and a
backing adapter. The burn names the adapter, so the user's own signed burn
picks the intent. A user who does both has two separate accounts.

If the pool refuses a deposit (stake bounds, pool full, already staked,
paused), the USDC waits in the account. It is never lost; the owner can
`stake_held` / `back_held` it or send it home.

## Rules the frontend must follow

1. **`destinationCaller` must be the adapter, always.** A burn with an empty
   `destinationCaller` can be completed by anyone directly at Circle, which
   mints to the adapter without creating a SAFU account: those funds are stuck.
   The adapter rejects such messages before consuming them, but it cannot stop
   someone else completing them directly.
2. **Ethereum: plain wallets (EOAs) only.** The owner is the address that
   burned. A contract wallet (e.g. a Safe) cannot produce the owner signature,
   so its account could never be used. Check `getCode(address)` is empty before
   allowing the burn.
3. **Cap the amount live** from the pool's settings (min/max stake, pool cap).
   Out-of-bounds deposits are held, not lost, but the user should not have to
   recover them.
4. **Signing format (`address_v2`).** Stellar now issues auth entries with
   `SOROBAN_CREDENTIALS_ADDRESS_V2`, whose signed payload also binds the
   account address (CAP-71). Build the payload with the SDK
   (`stellar_sdk.auth.authorize_entry` / the JS equivalent), never by hand.
   The contract only sees the 32-byte payload, so it works with either format.
   - EVM: the wallet `personal_sign`s the 32-byte payload. Signature =
     `OwnerSignature::Evm(r‖s‖v)`.
   - Solana: the wallet `signMessage`s the 32-byte payload. Signature =
     `OwnerSignature::Solana(sig)`.
5. **Solana deposits must carry the user's USDC token account as hook data.**
   CCTP mints to a token account, not a wallet. Burn with
   `deposit_for_burn_with_hook`, hook data = the user's USDC associated token
   account (32 bytes) followed by its bump (1 byte), exactly as
   `findAssociatedTokenPda` returns them (33 bytes total). The adapter
   recomputes the PDA from the burner and the burned mint and ignores anything
   else. The account saves the first valid one for good, and every payout goes
   there. No withdrawal call takes a destination, so nothing signed later can
   redirect money (pre-audit /cso H-1, 2026-09-24). A deposit without valid
   hook data still lands; the user just can't send home until some deposit
   carries it. EVM burns need no hook data (payouts always go to the owner).

## The last step on the home chain costs gas

Stellar → home burns leave `destinationCaller` empty, so **anyone** can
complete them on the home chain (`receiveMessage` on Ethereum's
MessageTransmitterV2, `receive_message` on Solana). Someone pays that gas:

- **Option A:** the user completes it from the frontend (needs a little ETH /
  SOL).
- **Option B:** a SAFU relayer completes it. On Solana this needs a dedicated
  Solana relayer key (KMS alias reserved: `safu-solana-relayer`, not created).

**DECIDED 2026-09-23 (founder): Option A.** The frontend hands the user a
normal `receiveMessage` / `receive_message` transaction on their home chain;
they approve and pay in their own wallet. No SAFU relayer on this leg, and
**`safu-solana-relayer` is dropped** — do not create it. Because
`destinationCaller` is empty here, an abandoned payout is not lost: anyone,
including SAFU ad hoc, can complete it later.

**The inbound leg is the opposite and stays SAFU-paid.** After Circle attests
a home-chain burn, a SAFU relayer calls `mint_and_stake` on Stellar and pays
the XLM fee — the user holds no Stellar key by design. Built in Phase 4
(Iris poll → `mint_and_stake`).

## Fees and decimals

- Stellar USDC has 7 decimals, CCTP 6. Inbound amounts are scaled ×10;
  outbound burns only whole canonical units, and any remainder below 1
  canonical unit stays in the account for the next send.
- Fast transfer (inbound only) charged 0.01% on testnet (2026-09-22); standard
  is free. Stellar outbound is always standard.
- Decimal config and fees are read live from Circle's TokenMessengerMinter,
  never hardcoded.

## Cost

Heaviest transaction (first deposit: Circle mint + account deploy + stake):
14.1M instructions, 10.6 MB, against testnet limits of 400M / 40 MB
(test `first_deposit_fits_network_limits`).

## Testnet (throwaway, 2026-09-22; abandon at B4)

| | |
|---|---|
| Pool (Circle USDC, no vault) | `CAE2RCDPZG55ZQT7FT7GYBCFTCSWHN4T3EBXYVYDOX2N3UK6EDZHB5DX` |
| Staking adapter | `CDJ4XO75RBZSDKNDPWCPYF6VFSU6335F77ZGPQGYX4P26FDIUCFCC7PH` (pre-backer build; redeploy for B4) |
| Circle MessageTransmitterV2 / TokenMessengerMinterV2 | `CBJ6MTCK…VVJY` / `CDNG7HXA…RTHP` |
| Circle testnet USDC SAC | `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |

Testnet USDC note: Blend's testnet pool uses a different USDC (`GATALT…`), so
testnet runs without the yield vault. Mainnet has one USDC; confirm Blend's
mainnet reserve is Circle's before turning the vault on.
