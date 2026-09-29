// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Vm} from "forge-std/Vm.sol";
import {MockLendingMarket} from "../src/MockLendingMarket.sol";

contract MockLendingMarketTest is Test {
    MockLendingMarket market;
    address alice = address(0xA11CE);
    address liquidator = makeAddr("liquidator");

    // $2,000/ETH, 8 decimals -- a round starting price for readable math.
    uint256 constant INITIAL_PRICE = 2_000 * 1e8;

    function setUp() public {
        market = new MockLendingMarket(INITIAL_PRICE);
        vm.deal(alice, 10 ether);
        vm.deal(liquidator, 1 ether);
    }

    // --- construction ---------------------------------------------------

    function test_constructor_sets_the_initial_price() public view {
        assertEq(market.priceE8(), INITIAL_PRICE);
    }

    function test_constructor_rejects_zero_price() public {
        vm.expectRevert("price must be positive");
        new MockLendingMarket(0);
    }

    // --- deposit / borrow -------------------------------------------------

    function test_deposit_records_collateral() public {
        vm.prank(alice);
        market.deposit{value: 1 ether}();
        (uint256 collateral, ) = market.positions(alice);
        assertEq(collateral, 1 ether);
    }

    function test_borrow_up_to_the_threshold_succeeds() public {
        vm.startPrank(alice);
        market.deposit{value: 1 ether}(); // $2,000 of collateral
        // 80% of $2,000 = $1,600 max debt (8 decimals)
        market.borrow(1_600 * 1e8);
        vm.stopPrank();
        (, uint256 debt) = market.positions(alice);
        assertEq(debt, 1_600 * 1e8);
    }

    function test_borrow_past_the_threshold_reverts() public {
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        vm.expectRevert("borrow would exceed the liquidation threshold");
        market.borrow(1_601 * 1e8);
        vm.stopPrank();
    }

    function test_borrow_with_no_collateral_reverts() public {
        vm.prank(alice);
        vm.expectRevert("no collateral deposited");
        market.borrow(1);
    }

    // --- isLiquidatable ----------------------------------------------------

    function test_position_at_the_threshold_is_not_yet_liquidatable() public {
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8); // exactly 80%
        vm.stopPrank();
        assertFalse(market.isLiquidatable(alice), "exactly at the threshold must not be liquidatable");
    }

    function test_price_drop_makes_a_healthy_position_liquidatable() public {
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8); // healthy at $2,000/ETH
        vm.stopPrank();
        assertFalse(market.isLiquidatable(alice));

        market.setPrice(1_999 * 1e8); // a genuine, tiny real move
        assertTrue(market.isLiquidatable(alice), "debt now exceeds 80% of the new collateral value");
    }

    // --- the wrongful-liquidation staging path itself -----------------------

    function test_a_wrongful_liquidation_can_be_staged_at_any_price() public {
        // Healthy position at the REAL price.
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8);
        vm.stopPrank();
        assertFalse(market.isLiquidatable(alice));

        // Force an obviously fake price -- this is the entire point of the
        // contract. A real market's oracle would never report this.
        uint256 fakePrice = 1 * 1e8; // $1/ETH
        market.setPrice(fakePrice);
        assertTrue(market.isLiquidatable(alice));

        vm.expectEmit(true, true, false, true);
        emit MockLendingMarket.Liquidated(alice, liquidator, 1 ether, 1_600 * 1e8, fakePrice, block.timestamp);

        vm.prank(liquidator);
        market.liquidate(alice);

        (uint256 collateral, uint256 debt) = market.positions(alice);
        assertEq(collateral, 0);
        assertEq(debt, 0);
    }

    function test_liquidation_emits_the_price_it_executed_at() public {
        // The ONE field the off-chain detector actually reads to judge
        // wrongful vs genuine -- pinned explicitly, not just checked as a
        // side effect of the event-equality assertion above.
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8);
        vm.stopPrank();

        uint256 stagedPrice = 500 * 1e8;
        market.setPrice(stagedPrice);

        vm.recordLogs();
        vm.prank(liquidator);
        market.liquidate(alice);

        Vm.Log[] memory logs = vm.getRecordedLogs();
        bool found = false;
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].topics[0] == keccak256("Liquidated(address,address,uint256,uint256,uint256,uint256)")) {
                (, , uint256 priceE8, ) = abi.decode(logs[i].data, (uint256, uint256, uint256, uint256));
                assertEq(priceE8, stagedPrice);
                found = true;
            }
        }
        assertTrue(found, "Liquidated event was not emitted");
    }

    function test_liquidate_reverts_on_a_healthy_position() public {
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8);
        vm.stopPrank();

        vm.expectRevert("position is not liquidatable at the current price");
        market.liquidate(alice);
    }

    function test_liquidation_seizes_all_collateral_to_the_liquidator() public {
        vm.startPrank(alice);
        market.deposit{value: 2 ether}();
        market.borrow(3_200 * 1e8); // 80% of $4,000
        vm.stopPrank();

        market.setPrice(1 * 1e8);
        uint256 balBefore = liquidator.balance;

        vm.prank(liquidator);
        market.liquidate(alice);

        assertEq(liquidator.balance, balBefore + 2 ether);
    }

    function test_liquidation_is_permissionless_matching_real_lending_markets() public {
        // Anyone can liquidate, not just an admin -- same convention Aave,
        // Compound and Blend all use, named explicitly in the contract doc.
        vm.startPrank(alice);
        market.deposit{value: 1 ether}();
        market.borrow(1_600 * 1e8);
        vm.stopPrank();
        market.setPrice(1 * 1e8);

        address randomCaller = address(0xBEEF);
        vm.prank(randomCaller);
        market.liquidate(alice); // does not revert
    }

    // --- setPrice is deliberately unrestricted ------------------------------

    function test_setPrice_has_no_access_control_by_design() public {
        vm.prank(address(0xDEAD));
        market.setPrice(1);
        assertEq(market.priceE8(), 1);
    }

    function test_setPrice_rejects_zero() public {
        vm.expectRevert("price must be positive");
        market.setPrice(0);
    }
}
