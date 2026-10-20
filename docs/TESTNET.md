# Testnet deployment

Deployed 2026-10-20 to Stellar testnet from tag `audit-freeze-2026-10-20` (commit `85b2be5`), the
commit under audit. Production time constants, production role keys. Every role was read back from
the chain after the deploy.

Stellar testnet has two USDC tokens: Circle's (used by CCTP) and Blend's (used by Blend's testnet
pool and DeFindex). Mainnet has one. So the same code runs as two testnet pools: one on Circle USDC
for staking, backing and claims across chains, and one on Blend's USDC for the yield vault.

## Pool 1: Circle USDC (cross-chain, claims)

| Contract | Address |
|---|---|
| protection-pool | `CBQ73BYNJJKA43G4RYAJN3LQ4CDCSPWCTZZKPGCZQS4EOVPCLBY5AQIW` |
| covered-registry | `CDE3QYEK6JQ55VKWBFOBGHB2HSDDL535OCTU7HUWISZ2IJTOTAD6YHWR` |
| cctp-adapter (stake) | `CBOPK7UXUEDOCEETEB6R646T3MA64XFJYZZDQ4ZU44FHYW2R7ENAPJLN` |
| cctp-adapter (back) | `CAKDPXGCRB32O5CX4W5JEEQNOSJJPVAFC36FQBQITTADML6DK4IL376F` |
| Asset | Circle testnet USDC `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` |
| Pool cap | 100,000 USDC ($10 to $100 per stake) |
| Vault | none, deploy ceiling 0 |

These addresses are also in [`config/pool.testnet.json`](../config/pool.testnet.json).

## Pool 2: Blend USDC (yield vault)

| Contract | Address |
|---|---|
| protection-pool | `CCC4ITFNHK6X7OI42ZP6S4472QF65OWA4VFAUCVBKABTW2XJYBYPTFXU` |
| DeFindex vault | `CAOM4ERP6XMFUXMT3DCU7CTZFC6KRWWHPTGZY2I43EWFJ7JSJWEEW42A` (fee 0, not upgradable) |
| Blend strategy | `CALLOM5I7XLQPPOPQMYAHUWW4N7O3JKT42KQ4ASEEVBXDJQNJOALFSUY` |
| DeFindex factory | `CDSCWE4GLNBYYTES2OCYDFQA2LLY4RBIAX6ZI32VSUXD7GO6HRPO4A32` |
| Asset | Blend testnet USDC `CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU` |
| Pool cap | 10,000 USDC ($1 to $10 per stake) |
| Deploy ceiling | 80% |

The vault was created and seeded by a separate wallet, then fully invested into the Blend strategy,
by `scripts/deploy/setup_vault.sh`. After the first manual deposit, a stake moved its share into the
vault automatically and the pool held at the 80% line.

## Roles (both pools)

| Role | Address | Key |
|---|---|---|
| Admin | `GDHYJFXJWDWIPAMH7TM3YLCATRIKIFJITH3OXIMTAXUELKIQXYH5W356` | local keystore |
| Co-signer | `GDCVV36HRHXAG3ZZNXPHNBGT2FMLESIFBMUFGZUHOZH7N5XGCXH3I2AG` | AWS KMS |
| Guardian | `GBON25AKIZETLWO7RM4M2GYR5OE5ZYQQ6V3H4DGZCMEUQ6OBY3TVBD5N` | AWS KMS, access key offline |
| Oracle | `GCZXLI5O27T3SKMCSKCR5NPE5WHQTFOALS3LRCERIDRMROSDNK3X6A3P` | AWS KMS |
| Registry writer | `GC3UMF22VAUKANSGWTVZX722ZHMAXPFVDWRKO7ETLEXMCEXVDGSCVUG2` | AWS KMS |
| Treasury | the admin address (testnet only) | |

Any change to a role, the vault, the treasury, the deploy ceiling or the code needs two of admin,
co-signer and guardian, then 7 days in public. Settings need admin and co-signer, then 7 days.

## WASM (sha256)

| Contract | Hash |
|---|---|
| protection_pool | `f81fa4f8d3cdd7c1503cd245269672058344a7ac76411f12566c7f8830e39d1b` |
| covered_registry | `c3e4cec972aec5c6529406c60f5d824ba1a82fc18db18dddd9dbd7494c6475f0` |
| cctp_adapter | `6324dbe7a98049a58f7602f050a21e41d4e5221b4cc2177f451fc6d13987e213` |
| safu_account | `75e4008e8c7d9c9fc017dfb7398f8e5e11a99aca4da3ac823c2fcc9d7e862c21` |

## Checks run on pool 1

Setup with the production keys (treasury, deploy ceiling, then reopening the pool), then a short smoke on the real
clocks: a minimum stake, a covered wallet registered by the KMS writer, a KMS-signed claim recorded
as waiting for day 90, an early release refused, a pool cap raise without the wait refused, a setting
and a governance change each refused before their 7 days, and a governance change cancelled by two
votes. 17 of 17 passed.
