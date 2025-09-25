// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.20;

import "./RollupV6.sol";

contract RollupV7 is RollupV6 {
    bool public mintsAndBurnsDisabled;

    event MintsAndBurnsStatusChanged(bool isDisabled);
    event MintsRefunded(
        address indexed recipient,
        bytes32[] commitments,
        uint256 totalAmount
    );
    event EmergencyFundsSent(address indexed recipient, uint256 amount);

    /**
     * @notice Initializes RollupV7 specific features.
     * @dev Sets the contract version to 7 and initializes mintsAndBurnsDisabled to false.
     * This function should be called by the proxy admin during an upgrade.
     */
    function initializeV7() public reinitializer(7) {
        version = 7;
        mintsAndBurnsDisabled = false;
    }

    /**
     * @notice Sets the disabled status for minting and burning operations.
     * @param _disabled True to disable mints and burns, false to enable.
     * Can only be called by the owner.
     */
    function setMintsAndBurnsDisabled(bool _disabled) public onlyOwner {
        mintsAndBurnsDisabled = _disabled;
        emit MintsAndBurnsStatusChanged(_disabled);
    }

    /**
     * @notice Refunds specified active mints to a recipient address.
     * @dev This function is intended for emergency use, typically when mints/burns are disabled.
     * It requires a list of mint commitments to be provided off-chain.
     * The USDC funds corresponding to these mints are transferred to the recipient.
     * Mints are deleted after refunding to prevent double processing.
     * @param _commitments An array of mint commitments to be refunded.
     * @param _recipient The address to receive the refunded USDC.
     * Can only be called by the owner.
     */
    function emergencyRefundSpecificMints(
        bytes32[] calldata _commitments,
        address _recipient
    ) public onlyOwner {
        require(
            mintsAndBurnsDisabled,
            "RollupV7: Mints and burns must be disabled for emergency refund"
        );
        require(
            _recipient != address(0),
            "RollupV7: Recipient cannot be zero address"
        );
        require(_commitments.length > 0, "RollupV7: No commitments to refund");

        uint256 totalRefundAmount = 0;
        for (uint i = 0; i < _commitments.length; i++) {
            bytes32 commitment = _commitments[i];
            uint256 amount = mints[commitment];

            if (amount > 0) {
                // Check if mint exists and has not been refunded/processed
                totalRefundAmount += amount;
                delete mints[commitment]; // Mark as refunded/processed
            }
        }

        require(
            totalRefundAmount > 0,
            "RollupV7: No valid mints found for refund or total amount is zero"
        );

        IERC20(usdc).transfer(_recipient, totalRefundAmount);
        emit MintsRefunded(_recipient, _commitments, totalRefundAmount);
    }

    /**
     * @notice Allows the owner to send a specific amount of USDC from the contract to a recipient.
     * @dev This function is intended for emergency use, typically when mints/burns are disabled.
     * @param _to The address to receive the USDC.
     * @param _amount The amount of USDC to send.
     * Can only be called by the owner.
     */
    function emergencySendFunds(address _to, uint256 _amount) public onlyOwner {
        require(
            mintsAndBurnsDisabled,
            "RollupV7: Mints and burns must be disabled for emergency send"
        );
        require(
            _to != address(0),
            "RollupV7: Recipient cannot be zero address"
        );
        require(_amount > 0, "RollupV7: Amount must be greater than zero");

        IERC20(usdc).transfer(_to, _amount);
        emit EmergencyFundsSent(_to, _amount);
    }

    function mint(
        bytes calldata proof,
        bytes32 commitment,
        bytes32 value,
        bytes32 source
    ) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.mint(proof, commitment, value, source);
    }

    function mintWithAuthorization(
        bytes calldata proof,
        bytes32 commitment,
        bytes32 value,
        bytes32 source,
        address from,
        uint256 validAfter,
        uint256 validBefore,
        bytes32 nonce,
        uint256 v,
        bytes32 r,
        bytes32 s,
        uint256 v2,
        bytes32 r2,
        bytes32 s2
    ) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.mintWithAuthorization(
            proof,
            commitment,
            value,
            source,
            from,
            validAfter,
            validBefore,
            nonce,
            v,
            r,
            s,
            v2,
            r2,
            s2
        );
    }

    function burn(
        address to,
        bytes calldata proof,
        bytes32 nullifer,
        bytes32 value,
        bytes32 source,
        bytes32 sig
    ) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.burn(to, proof, nullifer, value, source, sig);
    }

    function burnToAddress(
        bytes32 kind,
        bytes32 to,
        bytes calldata proof,
        bytes32 nullifier,
        bytes32 value,
        bytes32 source,
        bytes32 sig
    ) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.burnToAddress(kind, to, proof, nullifier, value, source, sig);
    }

    function burnToRouter(
        bytes32 kind,
        bytes32 msgHash,
        bytes calldata proof,
        bytes32 nullifier,
        bytes32 value,
        bytes32 source,
        bytes32 sig,
        address router,
        bytes calldata routerCalldata,
        address returnAddress
    ) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.burnToRouter(
            kind,
            msgHash,
            proof,
            nullifier,
            value,
            source,
            sig,
            router,
            routerCalldata,
            returnAddress
        );
    }

    function substituteBurn(bytes32 nullifier, uint256 amount) public override {
        require(
            !mintsAndBurnsDisabled,
            "RollupV7: Mints and burns are disabled"
        );
        super.substituteBurn(nullifier, amount);
    }
}
