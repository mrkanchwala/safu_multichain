// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Minimal subset of LayerZero EndpointV2, written out here so this contract has no third-party
/// dependency. Field order matches the deployed endpoint's ABI.
struct MessagingParams {
    uint32 dstEid;
    bytes32 receiver;
    bytes message;
    bytes options;
    bool payInLzToken;
}

struct MessagingFee {
    uint256 nativeFee;
    uint256 lzTokenFee;
}

struct MessagingReceipt {
    bytes32 guid;
    uint64 nonce;
    MessagingFee fee;
}

interface IEndpointV2 {
    function send(MessagingParams calldata p, address refundAddress)
        external
        payable
        returns (MessagingReceipt memory);

    function quote(MessagingParams calldata p, address sender) external view returns (MessagingFee memory);

    function setDelegate(address delegate) external;
}

/// Sends a covered-wallet registration to the Stellar registry-oapp.
/// Message layout (65 bytes): stakerHash(32) ++ chainId(1) ++ walletHash(32), see
/// contracts-layerzero/registry-oapp. chainId 1 = Ethereum.
///
/// Testnet demo sender. No inbound path: it cannot receive messages, and its only privileged
/// action is `owner` setting the destination, which is fixed at construction.
contract RegistrySender {
    IEndpointV2 public immutable endpoint;
    address public immutable owner;
    uint32 public immutable dstEid;
    bytes32 public immutable dstReceiver;

    uint8 public constant CHAIN_ETH = 1;
    /// executor lzReceive option: type 3, worker 1, length 17, option type 1, gas (uint128).
    uint128 public immutable receiveGas;

    event Registered(bytes32 indexed guid, bytes32 stakerHash, bytes32 walletHash);

    error NotOwner();

    constructor(address _endpoint, uint32 _dstEid, bytes32 _dstReceiver, uint128 _receiveGas) {
        endpoint = IEndpointV2(_endpoint);
        owner = msg.sender;
        dstEid = _dstEid;
        dstReceiver = _dstReceiver;
        receiveGas = _receiveGas;
        IEndpointV2(_endpoint).setDelegate(msg.sender);
    }

    function encodeMessage(bytes32 stakerHash, bytes32 walletHash) public pure returns (bytes memory) {
        return abi.encodePacked(stakerHash, CHAIN_ETH, walletHash);
    }

    function _options() internal view returns (bytes memory) {
        return abi.encodePacked(uint16(3), uint8(1), uint16(17), uint8(1), receiveGas);
    }

    function _params(bytes32 stakerHash, bytes32 walletHash) internal view returns (MessagingParams memory) {
        return MessagingParams(dstEid, dstReceiver, encodeMessage(stakerHash, walletHash), _options(), false);
    }

    function quote(bytes32 stakerHash, bytes32 walletHash) external view returns (uint256) {
        return endpoint.quote(_params(stakerHash, walletHash), address(this)).nativeFee;
    }

    function register(bytes32 stakerHash, bytes32 walletHash) external payable returns (bytes32 guid) {
        if (msg.sender != owner) revert NotOwner();
        MessagingReceipt memory r = endpoint.send{value: msg.value}(_params(stakerHash, walletHash), msg.sender);
        emit Registered(r.guid, stakerHash, walletHash);
        return r.guid;
    }
}
