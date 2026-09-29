# Architecture

## System

```mermaid
flowchart TB
    subgraph chains[Three covered chains]
        ETH[Ethereum / Sepolia<br/>ETH, USDC, USDT]
        SOL[Solana devnet<br/>SOL]
        XLM_C[Stellar testnet<br/>XLM]
    end

    subgraph offchain[Off-chain backend]
        SCANNER[Scanner<br/>drain detection, per chain]
        LIQ[Liquidation detector<br/>price-deviation check]
        REG[Covered-wallet registry<br/>ownership, pre-registration]
        REFLECTOR_CLIENT[Reflector client<br/>price reads + conversion]
        ORACLE[Throwaway oracle<br/>Ed25519 signing]
    end

    subgraph reflector[Reflector oracle network]
        RFL[(Mainnet price feed<br/>ETH, SOL, USDC, USDT, XLM)]
    end

    subgraph stellar[Stellar testnet]
        POOL[(Protection pool<br/>USDC-denominated<br/>forked from the live mainnet pool)]
        DEFINDEX[DeFindex vault]
        BLEND[(Blend USDC pool)]
    end

    ETH -->|tx hash / liquidation event| SCANNER
    SOL -->|tx hash / liquidation event| SCANNER
    XLM_C -->|tx hash / liquidation event| SCANNER

    SCANNER --> REG
    LIQ --> REG
    REG -->|ownership verified| REFLECTOR_CLIENT
    RFL --> REFLECTOR_CLIENT
    REFLECTOR_CLIENT -->|entitlement in USDC| ORACLE
    ORACLE -->|signed approval| POOL

    POOL -->|deploy| DEFINDEX
    DEFINDEX -->|route| BLEND
    BLEND -->|yield| DEFINDEX
    DEFINDEX -->|realised yield| POOL
    POOL -->|USDC payout| ETH
    POOL -->|USDC payout| SOL
    POOL -->|USDC payout| XLM_C
```

## Claim sequence: either attack type, one signer

```mermaid
sequenceDiagram
    participant U as Staker
    participant R as Registry
    participant D as Detector<br/>(scanner or liquidation check)
    participant P as Reflector + pricing
    participant O as Oracle (throwaway key)
    participant C as Protection pool

    U->>R: cite a registered wallet + tx / liquidation event
    R->>R: assert_covered(staker, chain, wallet, timestamp)
    alt not covered, or registered after the loss
        R-->>U: refused: no scan runs, no RPC spent
    else covered
        R->>D: run the verdict check
        D-->>D: drain verdict, or price-deviation verdict
        alt not eligible (clean, or a genuine liquidation)
            D-->>U: refused, no payout
        else eligible
            D->>P: measure the loss, snapshot the price
            P-->>D: entitlement in USDC
            D->>O: sign(wallet, tx_id, entitlement, tier, deadline)
            O-->>C: signed approval
            C->>C: verify signature, check tier ceiling
            C-->>U: entitlement reserved
            Note over C: 90-day gate, then the staker's own<br/>approve_claim, then a 7-day cooldown
            C-->>U: linear vest, USDC, to the frozen beneficiary
        end
    end
```

Both attack types, a wallet drain and a wrongful liquidation, produce the same shape of signed approval and go through the same contract path from here. The detector differs; nothing downstream of it does.

## Trust boundary

```mermaid
flowchart LR
    subgraph enforced[Enforced on-chain, fixed at deploy]
        A[Tier ceiling]
        B[Oracle signature verification]
        C[90-day gate, cooldown, vesting]
        D[Solvency + daily outflow caps]
    end

    subgraph trusted[Off-chain, Python, changeable]
        E[Which wallets are registered to which staker]
        F[Whether a liquidation was priced fairly]
        G[The actual entitlement amount]
        H[Which chain a loss happened on]
    end

    E --> G
    F --> G
    H --> G
    G --> B
```

The contract never learns what was lost, on which chain, or why a liquidation was ruled wrongful. It verifies one signed number against a ceiling it can never exceed. Everything that decides that number lives off-chain and can be corrected without touching the deployed contract, the same on-chain/off-chain split SAFU's mainnet pool already runs on.

## What's forked, and what never changes

The `protection-pool` contract here started as a copy of the Soroban pool already live on Stellar mainnet. That original stays at its frozen WASM hash; nothing in this repository ever writes to it. This version adds, among other changes, a yield-split mechanism (a growing index for the staker's share, an explicit counter for the protocol's), built and reviewed the same way the rest of the pool was, Rust unit tests covering both the new accounting and its interaction with every existing invariant.
