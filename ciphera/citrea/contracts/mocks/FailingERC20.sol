// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/**
 * Test-only ERC20 that silently returns false from transfer() when
 * a flag is set, without reverting. This mirrors the behavior of
 * "weird" tokens (blacklists, paused transfers, return-false-only
 * tokens) that RollupV1.executeBurnToAddress is designed to tolerate.
 *
 * Used by the Finding 3 test to verify that a failed burn payout
 * does NOT decrement currentTvl — which is the bug the success-gate
 * in verifyBurn prevents.
 */
contract FailingERC20 is ERC20 {
    bool public transfersFail;

    constructor() ERC20("Failing", "FAIL") {
        _mint(msg.sender, 1_000_000 ether);
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setTransfersFail(bool fail) external {
        transfersFail = fail;
    }

    function transfer(address to, uint256 amount)
        public
        override
        returns (bool)
    {
        if (transfersFail) {
            // Silently return false instead of reverting.
            // This is the non-reverting-but-unsuccessful path.
            return false;
        }
        return super.transfer(to, amount);
    }
}
