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

    // Self-review fix (H2): per-recipient blacklist. Used to test the
    // fee-sink DoS scenario where transfers to a specific address
    // revert while everything else flows normally.
    mapping(address => bool) public blacklisted;

    constructor() ERC20("Failing", "FAIL") {
        _mint(msg.sender, 1_000_000 ether);
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setTransfersFail(bool fail) external {
        transfersFail = fail;
    }

    function setBlacklisted(address who, bool flag) external {
        blacklisted[who] = flag;
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
        // Revert for blacklisted recipients — mirrors tokens that
        // refuse transfers to OFAC-blocked / frozen addresses. The
        // rollup's fee-routing path must survive this without
        // bricking the whole burn settlement.
        require(!blacklisted[to], "FailingERC20: recipient blacklisted");
        return super.transfer(to, amount);
    }
}
