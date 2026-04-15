// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

// Thin re-export of OZ's ERC1967Proxy so hardhat-viem can deploy it
// by artifact name in tests. The V2 test suite needs a proxy wrapper
// because RollupV1's constructor calls _disableInitializers() on the
// implementation — initialize() can only run on a proxy front-end.
import {ERC1967Proxy} from "@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol";

contract RollupV2TestProxy is ERC1967Proxy {
    constructor(address implementation, bytes memory data)
        ERC1967Proxy(implementation, data)
    {}
}
