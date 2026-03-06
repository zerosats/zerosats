// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.9;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";

contract BurnToAddressRouter {
    function burnToAddress(address usdcAddr, address to, uint256 value) public {
        IERC20(usdcAddr).transferFrom(msg.sender, to, value);
    }
}
