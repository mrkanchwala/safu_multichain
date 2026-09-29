# contracts-evm

Solidity contracts for the Sepolia side of the SAFU multichain demo, built and tested with Foundry.

## Contracts

| Contract | Purpose |
|---|---|
| `src/RegistrySender.sol` | Sends a covered-wallet registration from Ethereum (Sepolia) to the Stellar `registry-oapp` through LayerZero. The message is 65 bytes: staker hash, chain id, wallet hash. It has no dependency on third-party LayerZero packages and cannot receive messages. Deployed on Sepolia at `0xb90419cECCBDC783CcA72EB1FAd5dd08a8665CFF`. |
| `src/MockLendingMarket.sol` | A small standalone lending market used to stage a wrongful liquidation. Its price is a single settable value, so the demo can liquidate a position at a price the real market never saw. It emits the execution price in its `Liquidated` event, which is all the off-chain detector in `backend/liquidation.py` reads. |

`MockLendingMarket` holds one collateral asset (native ETH), keeps debt as an internal ledger, seizes all
collateral on liquidation, and has no interest, no partial liquidation and no access control on `setPrice`.
Those limits are intentional: none of them affect what the detector reads.

## Build and test

```
forge build
forge test
```

Tests live in `test/`: `RegistrySender.t.sol` and `MockLendingMarket.t.sol`.
