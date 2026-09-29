# Circle CCTP v2 contracts (test fixtures)

Fetched from Stellar **testnet** on 2026-09-22 with `stellar contract fetch`
(an on-chain read, nothing from GitHub). The adapter tests run these real
contracts, configured like testnet, with local test attesters.

| File | Testnet contract | sha256 |
|---|---|---|
| `message_transmitter.wasm` | `CBJ6MTCKKZG73PMDZCJMSFRD7DQEMI4FKDH7CGDSV4W6FHCRBCQAVVJY` | `8927f7389410044b35b1d3d0d7d42ea4ed0677dea18cb1bd89be4a980566c614` |
| `token_messenger_minter.wasm` | `CDNG7HXAPBWICI2E3AUBP3YZWZELJLYSB6F5CC7WLDTLTHVM74SLRTHP` | `a04c09f4bf064cfafb7e4e931752de15a216af1d59373bdd9d53908e7d29a9fe` |
| `fiat_token_admin.wasm` | `CCELIKMY7RQ3BQERWSOQHLIVYC5E3UHLTLDYIO2NVS5XGFEXYER5UWSB` | `aba81900bc82d24a6f5b66a6fee775bc51543f91dc89e824bd96ffcf3879dba1` |

Testnet settings mirrored in the tests (read 2026-09-22): local domain 27,
message version 1, max body 8192, 2-of-2 attesters, USDC decimals 7 local /
6 canonical, min fee 0, Sepolia TokenMessengerV2 `0x8fe6…2daa` on domain 0.
USDC SAC `CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA` (derived
independently from `USDC:GBBD47…` with `stellar contract id asset`).

Before mainnet, re-fetch the mainnet contracts, compare hashes, and re-run.

## Running the tests

The flow tests load the release WASMs, so build first:

```
stellar contract build
cargo test -p cctp-adapter
```
