// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/**
 * Test-only ERC20 that attempts to reenter a configured target
 * contract from inside its own `transfer()` hook. Used to verify
 * that RollupV1's verifyRollup properly rejects reentrant calls
 * via its `nonReentrant` modifier.
 *
 * Arming:
 *   1. deploy
 *   2. call `setAttack(target, payload)` once, where `payload` is
 *      the ABI-encoded calldata for the function to re-invoke on
 *      `target` (typically verifyRollup with any args).
 *   3. the next transfer will fire the callback exactly once; the
 *      flag self-clears so subsequent transfers behave normally.
 *
 * We ignore the callback's return — the test asserts on observable
 * state afterwards (contract balance, currentTvl) rather than on
 * the callback itself, because executeBurnToAddress swallows reverts
 * and returns `false` which is exactly the behavior we're verifying.
 */
contract ReentrantERC20 is ERC20 {
    address public attackTarget;
    bytes public attackPayload;
    bool public attackArmed;

    // True iff a reentrant call to `attackTarget` succeeded. If the
    // rollup's nonReentrant modifier is working, this stays false
    // after an armed transfer — the reentry reverts inside the
    // target, our low-level call captures the revert as ok=false,
    // and we record that fact for the test to assert on.
    bool public reentrantCallSucceeded;

    constructor() ERC20("Reentrant", "RNT") {
        _mint(msg.sender, 1_000_000 ether);
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setAttack(address target, bytes calldata payload) external {
        attackTarget = target;
        attackPayload = payload;
        attackArmed = true;
    }

    function _update(
        address from,
        address to,
        uint256 value
    ) internal override {
        super._update(from, to, value);

        // Trigger the reentrant call only once, after the transfer
        // has settled, and only when actually moving tokens to a
        // real recipient (skip mint/burn edges where from/to == 0).
        if (attackArmed && from != address(0) && to != address(0)) {
            attackArmed = false;
            // Low-level call so the token doesn't revert when the
            // target reverts (e.g. when ReentrancyGuard fires on
            // the reentrant verifyRollup call). We record the
            // outcome so the test can assert that the guard fired.
            (bool ok, ) = attackTarget.call(attackPayload);
            if (ok) {
                reentrantCallSucceeded = true;
            }
        }
    }
}
