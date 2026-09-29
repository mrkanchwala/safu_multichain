// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import "../src/RegistrySender.sol";

contract MockEndpoint {
    MessagingParams public last;
    uint256 public lastValue;
    address public lastRefund;
    address public delegate;

    function setDelegate(address d) external {
        delegate = d;
    }

    function quote(MessagingParams calldata, address) external pure returns (MessagingFee memory) {
        return MessagingFee(12345, 0);
    }

    function send(MessagingParams calldata p, address refund) external payable returns (MessagingReceipt memory) {
        last = p;
        lastValue = msg.value;
        lastRefund = refund;
        return MessagingReceipt(bytes32(uint256(0xabc)), 1, MessagingFee(msg.value, 0));
    }
}

contract RegistrySenderTest is Test {
    MockEndpoint ep;
    RegistrySender s;
    bytes32 constant RECV = bytes32(uint256(0x1234));

    function setUp() public {
        ep = new MockEndpoint();
        s = new RegistrySender(address(ep), 40600, RECV, 200_000);
    }

    function test_message_is_65_bytes_with_chain_id_in_the_middle() public view {
        bytes memory m = s.encodeMessage(bytes32(uint256(0xAA)), bytes32(uint256(0xBB)));
        assertEq(m.length, 65);
        assertEq(uint8(m[32]), 1);
    }

    // Same golden vector as the Rust and Python tests: 0xAA*32, chain 1, 0xBB*32.
    function test_golden_vector_matches_stellar_and_python() public view {
        bytes32 a = bytes32(bytes.concat(bytes32(hex"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")));
        bytes32 b = bytes32(hex"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
        bytes memory m = s.encodeMessage(a, b);
        assertEq(sha256(m), 0x85fc8051287bece2c6c5a9ce4ef6373093360ac0d1a81ffbb28b23b05b049ace);
    }

    function test_register_sends_to_configured_destination() public {
        bytes32 g = s.register{value: 1 ether}(bytes32(uint256(1)), bytes32(uint256(2)));
        assertEq(g, bytes32(uint256(0xabc)));
        (uint32 dst, bytes32 recv,,,) = ep.last();
        assertEq(dst, 40600);
        assertEq(recv, RECV);
        assertEq(ep.lastValue(), 1 ether);
        assertEq(ep.lastRefund(), address(this));
    }

    function test_options_encoding() public {
        s.register(bytes32(uint256(1)), bytes32(uint256(2)));
        (,,, bytes memory opts,) = ep.last();
        // type3 | worker 1 | len 17 | optType 1 | gas 200000
        assertEq(opts, abi.encodePacked(uint16(3), uint8(1), uint16(17), uint8(1), uint128(200_000)));
    }

    function test_only_owner_can_register() public {
        vm.prank(address(0xBEEF));
        vm.expectRevert(RegistrySender.NotOwner.selector);
        s.register(bytes32(uint256(1)), bytes32(uint256(2)));
    }

    function test_delegate_set_to_deployer() public view {
        assertEq(ep.delegate(), address(this));
    }
}
