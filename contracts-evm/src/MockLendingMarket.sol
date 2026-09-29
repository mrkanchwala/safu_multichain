// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

/// @title MockLendingMarket
/// @notice A minimal, standalone lending market for staging a wrongful
/// liquidation on Sepolia. Deliberately has NO integration with any SAFU
/// contract, oracle, or off-chain component, the wrongful-liquidation
/// detector (`backend/liquidation.py`) is protocol-agnostic by construction,
/// so this market only needs to produce the shape any lending market
/// produces: collateral deposited, debt borrowed, and a liquidation event
/// carrying the price it was executed at.
///
/// WHY THIS EXISTS INSTEAD OF INTEGRATING A REAL MARKET LIKE AAVE
/// -----------------------------------------------------------------
/// Staging a WRONGFUL liquidation means forcing a liquidation at a price the
/// real market never actually saw. Aave's Sepolia deployment reads its price
/// from an oracle we do not control, so it can only ever execute a genuine
/// liquidation. This market's price is a single settable value instead,
/// which is the entire point, it lets the demo stage the exact failure
/// mode being detected, not merely a real one.
///
/// WHAT MAKES THE LIQUIDATION "WRONGFUL", AND WHO DECIDES THAT
/// ---------------------------------------------------------------
/// This contract has no opinion on whether a liquidation was wrongful, it
/// only executes what its own (settable) price says. The wrongful/genuine
/// judgment happens entirely off-chain, in `backend/liquidation.py`, by
/// comparing THIS market's execution price against Reflector's independent
/// price history. A liquidation here is "wrongful" precisely when this
/// contract's settable price diverges from that independent reality, which
/// is exactly the scenario `setPrice` exists to construct.
///
/// SCOPE, DELIBERATELY MINIMAL
/// -------------------------------
/// One collateral asset (native ETH), one fixed borrowable "debt" unit
/// (an internal ledger, not a real second token, nothing here needs actual
/// USDC to move for the liquidation EVENT to be real and detectable).
/// Liquidation seizes 100% of collateral on any position below the
/// threshold; there is no partial-liquidation curve, no interest accrual,
/// no multi-asset support. None of that affects what the detector reads.
contract MockLendingMarket {
    /// @notice Emitted on every liquidation. `priceE8` is the ONLY field the
    /// off-chain detector actually needs beyond the borrower and amount,
    /// everything else (whether it was wrongful) is decided by comparing
    /// this value against an independent source, not by anything in this
    /// contract or this event.
    event Liquidated(
        address indexed borrower,
        address indexed liquidator,
        uint256 collateralSeized,
        uint256 debtRepaid,
        uint256 priceE8,
        uint256 timestamp
    );

    event Deposited(address indexed borrower, uint256 amount);
    event Borrowed(address indexed borrower, uint256 amount);
    event PriceSet(uint256 priceE8, address indexed setter);

    struct Position {
        uint256 collateral; // wei
        uint256 debt; // internal USD-like units, 8 decimals (matches priceE8's scale)
    }

    /// @dev 80%: a position is liquidatable once debt exceeds 80% of
    /// collateral value at the CURRENT settable price. Fixed, not
    /// configurable per-position: this market exists to produce one clean
    /// liquidation shape, not to model a real risk curve.
    uint256 public constant LIQUIDATION_THRESHOLD_BPS = 8_000;
    uint256 private constant BPS_DENOMINATOR = 10_000;

    /// @notice Price of 1 ETH in USD, 8 decimals (matches Chainlink/Reflector
    /// convention so a comparison against a real feed needs no rescaling).
    /// Settable by ANYONE: deliberately. This is a demo instrument, not a
    /// production oracle; restricting `setPrice` to an admin would only add
    /// a signer to manage for zero real protection, since the entire
    /// contract's premise is "this price is not to be trusted".
    uint256 public priceE8;

    mapping(address => Position) public positions;

    constructor(uint256 initialPriceE8) {
        require(initialPriceE8 > 0, "price must be positive");
        priceE8 = initialPriceE8;
        emit PriceSet(initialPriceE8, msg.sender);
    }

    function setPrice(uint256 newPriceE8) external {
        require(newPriceE8 > 0, "price must be positive");
        priceE8 = newPriceE8;
        emit PriceSet(newPriceE8, msg.sender);
    }

    /// @notice Deposit ETH as collateral for the caller's own position.
    function deposit() external payable {
        require(msg.value > 0, "deposit must be positive");
        positions[msg.sender].collateral += msg.value;
        emit Deposited(msg.sender, msg.value);
    }

    /// @notice Borrow against the caller's own collateral, up to the
    /// liquidation threshold at the CURRENT price. No debt asset actually
    /// moves: this is a ledger entry, sufficient to make the position
    /// liquidatable, which is all the demo needs.
    function borrow(uint256 amount) external {
        require(amount > 0, "borrow amount must be positive");
        Position storage pos = positions[msg.sender];
        require(pos.collateral > 0, "no collateral deposited");

        uint256 newDebt = pos.debt + amount;
        uint256 collateralValue = (pos.collateral * priceE8) / 1 ether;
        uint256 maxDebt = (collateralValue * LIQUIDATION_THRESHOLD_BPS) / BPS_DENOMINATOR;
        require(newDebt <= maxDebt, "borrow would exceed the liquidation threshold");

        pos.debt = newDebt;
        emit Borrowed(msg.sender, amount);
    }

    /// @notice Whether `borrower`'s position is liquidatable at the CURRENT
    /// (settable) price. Exposed so the demo can confirm a staged price
    /// change actually crosses the threshold before calling `liquidate`.
    function isLiquidatable(address borrower) public view returns (bool) {
        Position storage pos = positions[borrower];
        if (pos.collateral == 0) return false;
        uint256 collateralValue = (pos.collateral * priceE8) / 1 ether;
        uint256 maxDebt = (collateralValue * LIQUIDATION_THRESHOLD_BPS) / BPS_DENOMINATOR;
        return pos.debt > maxDebt;
    }

    /// @notice Liquidate `borrower`'s position at the CURRENT price.
    /// Permissionless, matching every real lending market's own convention
    /// (Aave, Compound, Blend), anyone can call it, which is also why this
    /// contract makes no claim about whether doing so was fair. Seizes all
    /// collateral and clears all debt; no partial-liquidation curve, by
    /// design (see contract-level doc comment on scope).
    function liquidate(address borrower) external {
        require(isLiquidatable(borrower), "position is not liquidatable at the current price");

        Position storage pos = positions[borrower];
        uint256 seized = pos.collateral;
        uint256 repaid = pos.debt;

        pos.collateral = 0;
        pos.debt = 0;

        emit Liquidated(borrower, msg.sender, seized, repaid, priceE8, block.timestamp);

        (bool ok, ) = msg.sender.call{value: seized}("");
        require(ok, "collateral transfer to liquidator failed");
    }
}
