// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.28;

import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";
import "@openzeppelin/contracts-upgradeable/access/OwnableUpgradeable.sol";
import "@openzeppelin/contracts-upgradeable/utils/ReentrancyGuardUpgradeable.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IVerifier} from "../noir/IVerifier.sol";

struct Mint {
    bytes32 note_kind;
    uint256 amount;
    bool spent;
}

struct Signature {
    bytes32 r;
    bytes32 s;
    uint v;
}

struct ValidatorSet {
    mapping(address => bool) validators;
    address[] validatorsArray;
    // The height at which this validator set becomes valid, inclusive
    uint256 validFrom;
}

// We can't return a mapping from a public function, so this struct is used for the public
// return valjue
struct PublicValidatorSet {
    address[] validators;
    uint256 validFrom;
}

// Verifiers
struct Verifier {
    IVerifier verifier;
    uint32 messages_length;
    bool enabled;
}

string constant NETWORK = "Ciphera";
uint64 constant NETWORK_LEN = 7;
uint256 constant MAX_FUTURE_BLOCKS = 2_592_000; // 30 days (~1 sec blocks)

contract RollupV1 is
    Initializable,
    OwnableUpgradeable,
    ReentrancyGuardUpgradeable
{
    using SafeERC20 for IERC20;
    event RollupVerified(uint256 indexed height, bytes32 root);
    event Minted(bytes32 indexed hash, bytes32 value, bytes32 note_kind);
    event ValidatorSetAdded(uint256 index, uint256 validFrom);
    event Burned(
        address indexed token,
        bytes32 indexed burn_hash,
        address indexed recipient,
        bool substitute,
        bool success
    );
    event BurnClaimed(
        address indexed substituteAddress,
        bytes32 indexed substituteBurnKey
    );
    event MintAdded(
        bytes32 indexed mint_hash,
        uint256 value,
        bytes32 note_kind
    );
    event VerifierAdded(
        bytes32 verificationKey,
        address zkVerifierAddress,
        uint32 messages_length
    );
    event VerifierRemoved(bytes32 verificationKey, address zkVerifierAddress);
    event ProverAdded(address indexed prover);
    event ProverRemoved(address indexed prover);
    event RootHashUpdated(bytes32 indexed oldRoot, bytes32 indexed newRoot);
    event EscrowManagerUpdated(address indexed old_escrow, address indexed new_escrow);

    // Since the Initializable._initialized version number is private, we need to keep track of it ourselves
    uint8 public version;

    // Verifiers
    mapping(bytes32 => Verifier) public zkVerifiers;
    bytes32[] public zkVerifierKeys;

    // Contracts
    IERC20 public token;

    // Core rollup values
    uint256 public blockHeight;
    bytes32 public rootHash;

    // Mint - mints are removed after the rollup validates them. Mint hash is hash of commitments.
    mapping(bytes32 => Mint) public mints;

    // Burn Substitutor - stores a mapping of paid out substituted burns, so they can be refunded
    // once the rollup completes the original burn
    // Composite key (hash + burnAddress + noteKind + amount) => substitute address
    mapping(bytes32 => address) public substitutedBurns;

    // Allowed Tokens
    mapping(bytes32 => address) tokens;

    // Actors
    mapping(address => uint) provers;

    // Validators
    ValidatorSet[] private validatorSets;
    uint256 private validatorSetIndex;

    // Burn substitutors
    mapping(address => bool) private burnSubstitutors;

    address public escrowManager;

    // =====================================================================
    // V2 STORAGE (APPENDED — DO NOT REORDER OR INSERT ABOVE THIS LINE)
    //
    // This contract has no `__gap`, so any storage added or reordered
    // above this block would shift slot assignments and corrupt state
    // in the proxy. All V2 additions must be appended *here*, in
    // append-only order. If you need to add more state in V3, append
    // it below this block — again, never above.
    // =====================================================================

    // --- Idea 2: Deposit caps -------------------------------------------
    // Per-mint cap bounds the worst-case loss from any single malicious
    // or buggy deposit. Global TVL cap bounds the total value at risk
    // across the protocol during bootstrap. `currentTvl` is a running
    // counter — incremented on deposit in mint()/mintClaimed() and
    // decremented on burn in verifyBurn() — so the cap check has
    // something to compare against. It is seeded from the live token
    // balance during initializeV2 so an already-populated contract
    // doesn't start from zero and let deposits balloon past the cap.
    uint256 public perMintCap;
    uint256 public globalTvlCap;
    uint256 public currentTvl;

    // --- Idea 3: Open proving (liveness escape hatch) -------------------
    // `lastVerifiedAt` is the block.timestamp of the last successful
    // verifyRollup. If `openProvingDelay` elapses without a proof
    // landing — i.e. the whitelisted provers have gone silent — then
    // `isProvingOpen()` flips to true, which (a) unlocks verifyRollup
    // for any caller via onlyProverOrOpen and (b) short-circuits
    // verifyValidatorSignatures so a dead validator set can no longer
    // brick the chain. Both gates must open together; opening just
    // one is security theater because the other would still revert.
    //
    // The ZK verifier remains the ONLY safety gate in escape mode.
    // That is intentional and sufficient: the ZK proof is the only
    // thing that ever actually protected funds. Validator signatures
    // provide consensus coordination, not state-transition safety.
    uint256 public lastVerifiedAt;
    uint256 public openProvingDelay;

    // --- Idea 4: Validator activation floor -----------------------------
    // Even with the owner moved to a timelock, we want the contract
    // itself to enforce a minimum notice window before a newly
    // scheduled validator set activates. This gives users a guaranteed
    // reaction time independent of whatever delay the governance
    // layer happens to be configured with. Measured in blocks because
    // that is the unit `validFrom` already uses.
    uint256 public validatorActivationMinDelayBlocks;

    // --- Ideas 5-8: Guardian + withdrawal pause -------------------------
    // Governance (ownership) sits behind a timelock. But a pause that
    // is also delayed is useless in an active incident — by the time
    // it lands on-chain, funds are already gone. The guardian role is
    // therefore an instant-acting authority with exactly one power:
    // setting `withdrawalsPaused`. It cannot rotate anything else,
    // cannot upgrade, cannot mint. Its single power is intentionally
    // narrow so handing it to a fast-acting multisig is low-risk.
    //
    // The pause is deliberately a *soft* pause: it blocks new burn
    // substitutions (substituteBurn) but NOT the settlement path
    // (verifyBurn). This way, already-substituted burns still pay
    // substitutors back when the rollup lands — we avoid stranding
    // operators who are mid-loan. If a truly adversarial scenario
    // requires a hard pause, verifyBurn can be extended later.
    address public guardian;
    bool public withdrawalsPaused;

    // --- Idea 9: Burn fee -----------------------------------------------
    // Fee = max(burnFeeFloor, value * burnFeeBps / 10000), clamped to
    // `value` so we never overcharge. Floor protects against dust
    // spam (where bps fees would round to zero); bps scales revenue
    // with large burns. The fee is shaved out of the user's payout
    // in verifyBurn and forwarded to `feeSink` as a separate transfer.
    //
    // `feeSink` is a dedicated address — NOT the protocol owner —
    // so fee ownership can be governed independently of protocol
    // upgrades. This matters for any future handover to a DAO or
    // treasury, where we don't want a single address to both
    // upgrade the contract AND drain the fee balance.
    //
    // To keep token flow balanced when a burn is substituted, the
    // substitutor pays the *post-fee* amount upfront and receives
    // the post-fee amount back when the rollup lands. The fee stays
    // in the contract and is routed to feeSink during verifyBurn.
    // This keeps the substitutor whole on the loan principal.
    uint256 public burnFeeFloor;
    uint256 public burnFeeBps;
    address public feeSink;

    // =====================================================================
    // END V2 STORAGE
    // =====================================================================

    // --- Events added in V2 ---------------------------------------------
    // Self-review fix (L1): emit both old and new values so subgraphs
    // can derive deltas without a separate RPC read. Matches the
    // pattern used by other V2 update events.
    event PerMintCapUpdated(uint256 oldCap, uint256 newCap);
    event GlobalTvlCapUpdated(uint256 oldCap, uint256 newCap);
    event OpenProvingDelayUpdated(uint256 oldDelay, uint256 newDelay);
    event ValidatorActivationDelayUpdated(uint256 oldDelay, uint256 newDelay);
    event GuardianUpdated(address indexed oldGuardian, address indexed newGuardian);
    event WithdrawalsPausedSet(bool paused);
    event BurnFeeUpdated(uint256 oldFloor, uint256 oldBps, uint256 newFloor, uint256 newBps);
    event FeeSinkUpdated(address indexed oldSink, address indexed newSink);
    event FeePaid(address indexed token, address indexed sink, uint256 amount);
    // Self-review fix (H2): emitted when fee routing to feeSink fails
    // (e.g. sink address blacklisted by token, receiver reverts).
    // The fee is left in the contract; governance can either fix the
    // sink configuration or deploy a V3 sweep function.
    event FeeStuck(address indexed token, uint256 amount);

    /// @custom:oz-upgrades-unsafe-allow constructor
    constructor() {
        _disableInitializers();
    }

    function initialize(
        address owner,
        address _escrowManager,
        address _tokenAddress,
        address _verifierAddress,
        address prover,
        address[] calldata initialValidators,
        bytes32 verifierKeyHash
    ) public initializer {
        version = 1;

        __Ownable_init(owner);

        token = IERC20(_tokenAddress);

        // Set the init aggregate verifier
        _setZkVerifierProperties(verifierKeyHash, _verifierAddress, 6 * 5);
        zkVerifierKeys.push(verifierKeyHash);

        provers[prover] = 1;

        _setValidators(0, initialValidators);

        // Empty merkle tree root hash constant (from pkg/contracts/src/empty_merkle_tree_root_hash.txt)
        _setRootHash(
            0x0577b5b4aa3eaba75b2a919d5d7c63b7258aa507d38e346bf2ff1d48790379ff
        );
        tokens[
            0x000200000000000013fb8d0c9d1c17ae5e40fff9be350f57840e9e66cd930000
        ] = _tokenAddress;
        burnSubstitutors[owner] = true;

        escrowManager = _escrowManager;
    }

    /**
     * @notice V2 initializer — one-shot upgrade init.
     * @dev See docs/rollup-v2-upgrade-notes.md for security and ops rationale.
     */
    function initializeV2(
        uint256 perMintCap_,
        uint256 globalTvlCap_,
        uint256 openProvingDelay_,
        uint256 validatorActivationMinDelayBlocks_,
        address guardian_,
        uint256 burnFeeFloor_,
        uint256 burnFeeBps_,
        address feeSink_
    ) external onlyOwner reinitializer(2) {
        // See docs: init is owner-gated to prevent non-atomic upgrade frontruns.
        require(guardian_ != address(0), "RollupV1: invalid guardian");
        require(feeSink_ != address(0), "RollupV1: invalid fee sink");
        require(openProvingDelay_ >= 1 days, "RollupV1: open proving delay too short");
        require(burnFeeBps_ <= 500, "RollupV1: burn fee bps too high");
        require(perMintCap_ > 0, "RollupV1: per-mint cap must be nonzero");
        require(globalTvlCap_ >= perMintCap_, "RollupV1: TVL cap < per-mint cap");
        // Bound prevents validator-rotation soft-brick (see docs).
        require(
            validatorActivationMinDelayBlocks_ < MAX_FUTURE_BLOCKS,
            "RollupV1: validator activation delay too large"
        );

        // Initialize ReentrancyGuard storage for V2 verify path.
        __ReentrancyGuard_init();

        perMintCap = perMintCap_;
        globalTvlCap = globalTvlCap_;

        // Seed from live balance so caps start from current state.
        currentTvl = token.balanceOf(address(this));

        lastVerifiedAt = block.timestamp;
        openProvingDelay = openProvingDelay_;

        validatorActivationMinDelayBlocks = validatorActivationMinDelayBlocks_;

        guardian = guardian_;
        withdrawalsPaused = false;

        burnFeeFloor = burnFeeFloor_;
        burnFeeBps = burnFeeBps_;
        feeSink = feeSink_;

        version = 2;
    }

    modifier onlyProver() {
        require(provers[msg.sender] == 1, "You are not a prover");
        _;
    }

    function isProvingOpen() public view returns (bool) {
        // Pre-init guard for V2 config window; see docs.
        if (openProvingDelay == 0) return false;
        return block.timestamp >= lastVerifiedAt + openProvingDelay;
    }

    modifier onlyProverOrOpen() {
        require(
            provers[msg.sender] == 1 || isProvingOpen(),
            "RollupV1: not prover and proving not open"
        );
        _;
    }

    // --- V2: Guardian (Ideas 5-8) ---------------------------------------
    modifier onlyGuardian() {
        require(msg.sender == guardian, "RollupV1: not guardian");
        _;
    }

    function addProver(address prover) public onlyOwner {
        provers[prover] = 1;
        emit ProverAdded(prover);
    }

    function removeProver(address prover) public onlyOwner {
        require(provers[prover] == 1, "Address is not a prover");
        provers[prover] = 0;
        emit ProverRemoved(prover);
    }

    modifier onlyBurnSubstitutor() {
        require(
            burnSubstitutors[msg.sender] == true,
            "RollupV1: You are not a burn substitutor"
        );
        _;
    }

    modifier onlyEscrowManager() {
        require(msg.sender == escrowManager, "RollupV1: Only escrow manager");
        _;
    }

    function _setZkVerifierProperties(
        bytes32 keyHash,
        address verifierAddress,
        uint32 messagesLength
    ) internal {
        zkVerifiers[keyHash].verifier = IVerifier(verifierAddress);
        zkVerifiers[keyHash].messages_length = messagesLength;
        zkVerifiers[keyHash].enabled = true;
    }

    function addZkVerifier(
        bytes32 verificationKeyHash,
        address verifierAddress,
        uint32 messages_length
    ) public onlyOwner {
        require(
            verifierAddress != address(0),
            "RollupV1: Invalid zk verifier address"
        );
        require(
            verifierAddress.code.length > 0,
            "RollupV1: ZK verifier is not a contract"
        );
        // Add to verifier keys if not enabled
        if (!zkVerifiers[verificationKeyHash].enabled) {
            zkVerifierKeys.push(verificationKeyHash);
        }
        _setZkVerifierProperties(
            verificationKeyHash,
            verifierAddress,
            messages_length
        );
        emit VerifierAdded(
            verificationKeyHash,
            verifierAddress,
            messages_length
        );
    }

    function removeZkVerifier(bytes32 verificationKeyHash) public onlyOwner {
        require(
            zkVerifiers[verificationKeyHash].enabled,
            "RollupV1: ZK verifier does not exist"
        );
        address verifierAddress = address(
            zkVerifiers[verificationKeyHash].verifier
        );
        delete zkVerifiers[verificationKeyHash];
        // Find and remove the verifierKey from verifierKeys array
        uint256 length = zkVerifierKeys.length;
        for (uint256 i = 0; i < length; i++) {
            if (zkVerifierKeys[i] == verificationKeyHash) {
                // Move the last element to this position and remove the last element
                zkVerifierKeys[i] = zkVerifierKeys[length - 1];
                zkVerifierKeys.pop();
                break;
            }
        }
        emit VerifierRemoved(verificationKeyHash, verifierAddress);
    }

    function getZkVerifier(
        bytes32 verificationKeyHash
    ) public view returns (address, uint32) {
        require(
            zkVerifiers[verificationKeyHash].enabled,
            "RollupV1: ZK verifier does not exist"
        );
        return (
            address(zkVerifiers[verificationKeyHash].verifier),
            zkVerifiers[verificationKeyHash].messages_length
        );
    }

    function addBurnSubstitutor(address burnSubstitutor) public onlyOwner {
        burnSubstitutors[burnSubstitutor] = true;
    }

    function removeBurnSubstitutor(address burnSubstitutor) public onlyOwner {
        burnSubstitutors[burnSubstitutor] = false;
    }

    function _setRootHash(bytes32 newRoot) internal {
        bytes32 oldRoot = rootHash;
        rootHash = newRoot;
        emit RootHashUpdated(oldRoot, newRoot);
    }

    // Kept for ABI compatibility, intentionally disabled in V2 (see docs).
    function setRoot(bytes32 /* newRoot */) public pure {
        revert("RollupV1: setRoot disabled in V2");
    }

    function currentRootHash() public view returns (bytes32) {
        return rootHash;
    }

    // Alias-only and pre-V2-only; see docs for cap-accounting rationale.
    function addToken(bytes32 noteKind, address tokenAddress) public onlyOwner {
        require(version < 2, "RollupV1: addToken disabled in V2");
        require(
            tokens[noteKind] == address(0),
            "RollupV1: Token already exists"
        );
        require(
            tokenAddress == address(token),
            "RollupV1: addToken must alias primary token"
        );

        tokens[noteKind] = tokenAddress;
    }

    // =====================================================================
    // V2 setters — all `onlyOwner`, which means timelocked once ownership
    // has been transferred to a TimelockController. The guardian has its
    // own instant-acting setter (setWithdrawalsPaused) below; everything
    // else here requires going through the governance delay.
    // =====================================================================

    // --- Idea 2: Deposit caps -------------------------------------------
    function setPerMintCap(uint256 newCap) external onlyOwner {
        require(newCap > 0, "RollupV1: per-mint cap must be nonzero");
        require(newCap <= globalTvlCap, "RollupV1: per-mint cap > TVL cap");
        emit PerMintCapUpdated(perMintCap, newCap);
        perMintCap = newCap;
    }

    // Intentionally allows cap < currentTvl to freeze growth; see docs.
    function setGlobalTvlCap(uint256 newCap) external onlyOwner {
        require(newCap >= perMintCap, "RollupV1: TVL cap < per-mint cap");
        emit GlobalTvlCapUpdated(globalTvlCap, newCap);
        globalTvlCap = newCap;
    }

    // --- Idea 3: Open proving delay -------------------------------------
    function setOpenProvingDelay(uint256 newDelay) external onlyOwner {
        // 1 day floor matches the floor in initializeV2; prevents
        // governance from setting a near-zero delay that would make
        // escape mode trigger after a single missed block.
        require(newDelay >= 1 days, "RollupV1: open proving delay too short");
        emit OpenProvingDelayUpdated(openProvingDelay, newDelay);
        openProvingDelay = newDelay;
    }

    // --- Idea 4: Validator activation floor -----------------------------
    function setValidatorActivationMinDelayBlocks(uint256 newDelayBlocks)
        external
        onlyOwner
    {
        // Must stay below MAX_FUTURE_BLOCKS; see docs.
        require(
            newDelayBlocks < MAX_FUTURE_BLOCKS,
            "RollupV1: validator activation delay too large"
        );
        emit ValidatorActivationDelayUpdated(
            validatorActivationMinDelayBlocks,
            newDelayBlocks
        );
        validatorActivationMinDelayBlocks = newDelayBlocks;
    }

    // --- Ideas 5-8: Guardian + withdrawal pause -------------------------
    function setGuardian(address newGuardian) external onlyOwner {
        require(newGuardian != address(0), "RollupV1: invalid guardian");
        emit GuardianUpdated(guardian, newGuardian);
        guardian = newGuardian;
    }

    /**
     * @notice Pause or unpause new withdrawals. Instant, guardian-only.
     *
     * Soft pause semantics: this blocks new calls to substituteBurn()
     * but does NOT block verifyBurn(). In-flight substituted burns
     * continue to settle so substitutors don't get stranded mid-loan.
     *
     * If a hard pause is ever needed (an active exploit, not a
     * precaution), add the same check to verifyBurn in a follow-up
     * upgrade — intentionally not included here because the soft
     * pause is almost always what you want and the hard pause has
     * more failure modes for honest users and substitutors.
     */
    function setWithdrawalsPaused(bool paused_) external onlyGuardian {
        withdrawalsPaused = paused_;
        emit WithdrawalsPausedSet(paused_);
    }

    // --- Idea 9: Burn fee -----------------------------------------------
    function setBurnFee(uint256 newFloor, uint256 newBps) external onlyOwner {
        // 500 bps (5%) hard cap prevents governance from silently
        // imposing predatory withdrawal fees. If the protocol ever
        // wants a fee above 5%, that requires a contract upgrade,
        // which users can observe and react to.
        require(newBps <= 500, "RollupV1: burn fee bps too high");
        emit BurnFeeUpdated(burnFeeFloor, burnFeeBps, newFloor, newBps);
        burnFeeFloor = newFloor;
        burnFeeBps = newBps;
    }

    function setFeeSink(address newSink) external onlyOwner {
        require(newSink != address(0), "RollupV1: invalid fee sink");
        emit FeeSinkUpdated(feeSink, newSink);
        feeSink = newSink;
    }

    // --- Idea 9: Internal fee helper ------------------------------------
    /**
     * @dev Computes the fee charged on a burn of `value` wei.
     *
     * fee = max(burnFeeFloor, value * burnFeeBps / 10000),
     * clamped to `value` so we never try to charge more than the
     * burn itself is worth (which would underflow the payout).
     *
     * Edge cases:
     *  - If the fee would consume the entire burn, the payout is 0.
     *    The user gets nothing but the fee is still routed to the
     *    sink. We could revert instead, but that would let a fee
     *    misconfiguration brick verifyRollup; returning 0 keeps the
     *    protocol live and lets governance fix the fee afterwards.
     */
    function computeBurnFee(uint256 value) public view returns (uint256) {
        uint256 bpsFee = (value * burnFeeBps) / 10000;
        uint256 fee = bpsFee >= burnFeeFloor ? bpsFee : burnFeeFloor;
        return fee > value ? value : fee;
    }

    function noteKindTokenAddress(
        bytes32 noteKind
    ) public view returns (address) {
        return tokens[noteKind];
    }

    function DOMAIN_SEPARATOR() public view returns (bytes32) {
        return
                        keccak256(
                abi.encode(
                    keccak256(
                        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"
                    ),
                    keccak256(bytes("Rollup")),
                    keccak256(bytes("1")),
                    block.chainid,
                    address(this)
                )
            );
    }

    /////////////////
    //
    // VERIFY
    //
    ///////////

    // V2 liveness escape + reentrancy hardening; see docs.
    function verifyRollup(
        uint256 height,
        bytes32 verificationKeyHash,
        bytes calldata aggrProof,
    // oldRoot, newRoot, commitHash, <messages_length>, 16x kzg
        bytes32[] calldata publicInputs,
        bytes32 otherHashFromBlockHash,
        Signature[] calldata signatures
    ) public onlyProverOrOpen nonReentrant {
        require(
            zkVerifiers[verificationKeyHash].enabled,
            "RollupV1: ZK verifier not allowed"
        );

        require(
            publicInputs.length ==
            zkVerifiers[verificationKeyHash].messages_length + 3,
            "RollupV1: Invalid publicInputs length"
        );

        bytes32 oldRoot = publicInputs[0];
        bytes32 newRoot = publicInputs[1];
        bytes32 commitHash = publicInputs[2];

        verifyRootHash(oldRoot);

        verifyCommitHash(commitHash);

        verifyValidatorSignatures(
            newRoot,
            height,
            otherHashFromBlockHash,
            signatures
        );

        verifyAllMessages(
        // Skip the first 3 public inputs as these are never messages
            3,
            publicInputs,
            zkVerifiers[verificationKeyHash].messages_length,
            height
        );

        require(
            zkVerifiers[verificationKeyHash].verifier.verify(
                aggrProof,
                publicInputs
            ),
            "RollupV1: ZK proof verification failed"
        );

        rootHash = newRoot;
        require(
            height > blockHeight,
            "RollupV1: New block height must be greater than current"
        );
        blockHeight = height;

        // V2: reset the "last activity" timer so `isProvingOpen()`
        // returns false again. This is what keeps escape mode from
        // staying open during normal operation — every successful
        // proof closes the window for another `openProvingDelay`.
        lastVerifiedAt = block.timestamp;

        emit RollupVerified(height, newRoot);
    }

    function verifyRootHash(bytes32 expectedRoot) internal view {
        require(
            expectedRoot == rootHash,
            "RollupV1: Root hash verification failed"
        );
    }

    // Placeholder for asserting the commit hash is stored on Celestia
    function verifyCommitHash(bytes32 commitHash) internal {}

    function verifyAllMessages(
        uint skipCount,
        bytes32[] calldata publicInputs,
        uint32 messages_length,
        uint256 height
    ) internal {
        // Skip the first 4 (as they are processed separately)
        uint i = skipCount;
        uint end = skipCount + messages_length;
        while (i < end) {
            i = verifyMessages(i, publicInputs, height);
        }
    }

    function verifyMessages(
        uint index,
        bytes32[] calldata publicInputs,
        uint256 height
    ) internal returns (uint) {
        // Get the kind from public input at index
        uint256 kind = uint256(publicInputs[index]);

        if (kind == 0) {
            return index + 1;
        } else if (kind == 2) {
            // Mint
            return verifyMint(index, publicInputs);
        } else if (kind == 3) {
            // Burn
            return verifyBurn(index, publicInputs, height);
        } else {
            // Not allowed
            revert("RollupV1: Invalid message kind");
        }
    }

    function verifyMint(
        uint i,
        bytes32[] calldata messages
    ) internal returns (uint) {
        bytes32 note_kind = messages[i + 1];
        bytes32 value = messages[i + 2];
        bytes32 hash = messages[i + 3];

        require(
            mints[hash].amount == uint256(value),
            "RollupV1: Mint value invalid"
        );
        require(
            mints[hash].note_kind == note_kind,
            "RollupV1: Mint note kind invalid"
        );
        require(mints[hash].spent == false, "RollupV1: Mint already spent");
        mints[hash].spent = true;

        emit Minted(hash, value, note_kind);

        return i + 4;
    }

    function verifyBurn(
        uint i,
        bytes32[] calldata messages,
        uint256 height
    ) internal returns (uint) {
        bytes32 note_kind = messages[i + 1];
        uint256 value = uint256(messages[i + 2]);
        bytes32 hash = messages[i + 3];
        address burn_addr = bytes32ToAddress(messages[i + 4]);

        address _token = tokens[note_kind];

        bytes32 substituteBurnKey = getSubstituteBurnKey(
            hash,
            burn_addr,
            note_kind,
            value,
            height
        );
        address substitutor = substitutedBurns[substituteBurnKey];

        // Same fee schedule for substituted and direct burns.
        uint256 fee = computeBurnFee(value);
        uint256 payout = value - fee;

        // Only settle fee/tvl deltas if payout transfer succeeds.
        bool success;
        if (substitutor != address(0)) {
            success = executeBurn(_token, substitutor, hash, payout, false);
        } else {
            success = executeBurn(_token, burn_addr, hash, payout, false);
        }

        if (success) {
            // Non-reverting fee routing; if stuck, keep fee in contract.
            uint256 tokensLeavingContract = payout;

            if (fee > 0) {
                if (feeSink != address(0)) {
                    bool feeOk = _tryTransferFee(_token, feeSink, fee);
                    if (feeOk) {
                        tokensLeavingContract = value;
                        emit FeePaid(_token, feeSink, fee);
                    } else {
                        emit FeeStuck(_token, fee);
                    }
                } else {
                    // Sink unset; fee stays in contract.
                    emit FeeStuck(_token, fee);
                }
            }

            // Clamp handles legacy pre-V2 accounting edge cases.
            if (currentTvl >= tokensLeavingContract) {
                currentTvl -= tokensLeavingContract;
            } else {
                currentTvl = 0;
            }
        }

        return i + 5;
    }

    function bytes32ToAddress(
        bytes32 _bytes32
    ) internal pure returns (address) {
        return address(uint160(uint256(_bytes32)));
    }

    /**
     * @dev Helper function to generate composite key for substitutedBurns mapping
     * @param hash The burn hash
     * @param burnAddress The burn address
     * @param noteKind The note kind
     * @param amount The amount
     * @param burnBlockHeight The block height
     * @return The composite key for the mapping
     */
    function getSubstituteBurnKey(
        bytes32 hash,
        address burnAddress,
        bytes32 noteKind,
        uint256 amount,
        uint256 burnBlockHeight
    ) internal pure returns (bytes32) {
        return
                        keccak256(
                abi.encode(hash, burnAddress, noteKind, amount, burnBlockHeight)
            );
    }

    /////////////////
    //
    // BURNS
    //
    ///////////

    function executeBurn(
        address token,
        address recipient,
        bytes32 burn_hash,
        uint256 value,
        bool substitute
    ) internal returns (bool) {
        bool success = executeBurnToAddress(token, recipient, value);
        emit Burned(token, burn_hash, recipient, substitute, success);
        return success;
    }

    function executeBurnToAddress(
        address token,
        address recipient,
        uint256 value
    ) internal returns (bool) {
        (bool success, bytes memory returndata) = token.call(
            abi.encodeCall(IERC20.transfer, (recipient, value))
        );
        if (!success) {
            return false;
        }
        if (returndata.length != 0) {
            bool func_return = abi.decode(returndata, (bool));
            if (!func_return) {
                return false;
            }
        }
        return true;
    }

    // Non-reverting fee transfer helper for settlement liveness.
    function _tryTransferFee(
        address _token,
        address to,
        uint256 amount
    ) internal returns (bool) {
        (bool callOk, bytes memory returndata) = _token.call(
            abi.encodeCall(IERC20.transfer, (to, amount))
        );
        if (!callOk) return false;
        if (returndata.length != 0) {
            bool funcReturn = abi.decode(returndata, (bool));
            if (!funcReturn) return false;
        }
        return true;
    }

    function wasBurnSubstituted(
        address burn_address,
        bytes32 note_kind,
        bytes32 hash,
        uint256 amount,
        uint256 burnBlockHeight
    ) public view returns (bool) {
        bytes32 substituteBurnKey = getSubstituteBurnKey(
            hash,
            burn_address,
            note_kind,
            amount,
            burnBlockHeight
        );
        return substitutedBurns[substituteBurnKey] != address(0);
    }

    function substituteBurn(
        address burnAddress,
        bytes32 note_kind,
        bytes32 hash,
        uint256 amount,
        uint256 burnBlockHeight
    ) public onlyBurnSubstitutor {
        // V2 (Ideas 5-8): guardian-triggered soft pause.
        // We only block *new* substitutions. Already-substituted
        // burns continue to settle via verifyBurn so operators
        // who have money in flight don't get stranded.
        require(!withdrawalsPaused, "RollupV1: withdrawals paused");

        substituteBurnTo(
            burnAddress,
            msg.sender,
            note_kind,
            hash,
            amount,
            burnBlockHeight
        );
    }

    function substituteBurnTo(
        address burnAddress,
        address substituteAddress,
        bytes32 note_kind,
        bytes32 hash,
        uint256 amount,
        uint256 burnBlockHeight
    ) private {
        bytes32 substituteBurnKey = getSubstituteBurnKey(
            hash,
            burnAddress,
            note_kind,
            amount,
            burnBlockHeight
        );
        require(
            substitutedBurns[substituteBurnKey] == address(0),
            "RollupV1: Burn already substituted"
        );
        require(
            blockHeight < burnBlockHeight,
            "RollupV1: Block height already rolled up"
        );

        address _token = tokens[note_kind];
        require(_token != address(0), "RollupV1: Token not found for note kind");

        // V2 (Idea 9): burn-fee accounting for substituted burns.
        //
        // Keeping token balance math balanced while a burn is
        // substituted requires the substitutor to pre-pay only the
        // post-fee amount — otherwise the substitutor would end up
        // eating the fee on top of their own spread.
        //
        // Flow:
        //  1. Substitutor transfers `amount - fee` into this contract.
        //  2. Contract pays user `amount - fee`.
        //  3. On rollup proof, substitutor gets `amount - fee` back.
        //  4. The `fee` portion comes from the original mint's
        //     balance already held by this contract, and is routed
        //     to feeSink inside verifyBurn.
        //
        // The substitutedBurns key still uses the gross `amount` so
        // it matches the prover-side circuit output (which proves
        // the unadjusted burn value).
        uint256 fee = computeBurnFee(amount);
        uint256 payout = amount - fee;

        IERC20(_token).safeTransferFrom(
            substituteAddress,
            address(this),
            payout
        );

        bool success = executeBurn(_token, burnAddress, hash, payout, true);
        require(success, "RollupV1: Burn failed");

        // This will be returned to the msg.sender when the rollup block for it is submitted
        substitutedBurns[substituteBurnKey] = substituteAddress;
    }

    /////////////////
    //
    // MINTS
    //
    ///////////

    function getMint(bytes32 hash) public view returns (Mint memory) {
        return mints[hash];
    }

    // Function for EscrowManager only. As of Mar 2026 the best working solution
    function mintClaimed(
        bytes32 mint_hash,
        uint256 value,
        bytes32 note_kind
    ) public onlyEscrowManager {
        if (mints[mint_hash].amount != 0) {
            revert("RollupV1: Mint already exists");
        }

        // V2 (Idea 2): deposit caps. Applied here AS WELL AS in mint()
        // because `mintClaimed` is the escrow-manager path that does
        // not go through safeTransferFrom — the funds were already
        // collected off-chain by the EscrowManager. Without a cap
        // here, a compromised (or buggy) EscrowManager could mint
        // arbitrarily large amounts onto L2 regardless of how the
        // user-facing `mint()` is gated.
        require(value <= perMintCap, "RollupV1: exceeds per-mint cap");
        require(
            currentTvl + value <= globalTvlCap,
            "RollupV1: TVL cap reached"
        );
        currentTvl += value;

        address tokenAddress = tokens[note_kind];
        require(
            tokenAddress != address(0),
            "RollupV1: Token not found for note kind"
        );
        mints[mint_hash] = Mint({
            note_kind: note_kind,
            amount: value,
            spent: false
        });

        emit MintAdded(mint_hash, value, note_kind);
    }

    /**
     * @notice Record an off-chain substitution from the escrow manager.
     * @dev In V2, settlement returns `value - fee` on this path (see docs).
     */
    function burnClaimed(
        address substituteAddress,
        bytes32 substituteBurnKey
    ) public onlyEscrowManager {
        // This will be returned to the msg.sender when the rollup block for it is submitted
        substitutedBurns[substituteBurnKey] = substituteAddress;
        emit BurnClaimed(substituteAddress, substituteBurnKey);
    }

    // Anyone can call mint, although this is likely to be performed on behalf of the user
    // as they may not have gas to pay for the txn
    function mint(bytes32 mint_hash, bytes32 value, bytes32 note_kind) public {
        if (mints[mint_hash].amount != 0) {
            revert("RollupV1: Mint already exists");
        }

        // V2 (Idea 2): deposit caps.
        // Check BEFORE the safeTransferFrom so a capped-out deposit
        // doesn't cost the caller any token transfers or approvals.
        uint256 v = uint256(value);
        require(v <= perMintCap, "RollupV1: exceeds per-mint cap");
        require(currentTvl + v <= globalTvlCap, "RollupV1: TVL cap reached");
        currentTvl += v;

        address tokenAddress = tokens[note_kind];
        require(
            tokenAddress != address(0),
            "RollupV1: Token not found for note kind"
        );

        // Take the money from the external account, sender must have been previously
        // approved as per the ERC20 standard
        IERC20(tokenAddress).safeTransferFrom(
            msg.sender,
            address(this),
            v
        );

        // Add mint to pending mints, this still needs to be verifier with the verifyBlock,
        // but Solid validators will check that this commitment exists in the mint map before
        // accepting the mint txn into a block
        mints[mint_hash] = Mint({
            note_kind: note_kind,
            amount: v,
            spent: false
        });

        emit MintAdded(mint_hash, v, note_kind);
    }

    /////////////////
    //
    // VALIDATORS
    //
    ///////////
    function verifyValidatorSignatures(
        bytes32 newRoot,
        uint256 height,
        bytes32 otherHashFromBlockHash,
        Signature[] calldata signatures
    ) internal {
        // V2 (Idea 3): escape mode short-circuit.
        //
        // If the prover-inactivity timer has expired, we skip the
        // signature requirement entirely. Rationale: opening the
        // modifier `onlyProverOrOpen` without also skipping the sig
        // check would be security theater, because a user without
        // access to the (presumed-dead) validator set could never
        // collect enough signatures to pass the (2/3)+1 threshold.
        //
        // This is safe because validator signatures provide
        // *coordination*, not *safety*. The ZK verifier below this
        // function is what actually proves the state transition is
        // valid. As long as that succeeds, the new root is correct.
        if (isProvingOpen()) {
            return;
        }

        updateValidatorSetIndex(height);
        ValidatorSet storage validatorSet = getValidators();

        require(signatures.length > 0, "RollupV1: No signatures");

        uint minValidators = (validatorSet.validatorsArray.length * 2) / 3 + 1;
        require(
            signatures.length >= minValidators,
            "RollupV1: Not enough signatures from validators to verify block"
        );

        bytes32 sigHash = getSignatureMessageHash(
            newRoot,
            height,
            otherHashFromBlockHash
        );

        address previous = address(0);
        for (uint i = 0; i < signatures.length; i++) {
            Signature calldata signature = signatures[i];
            address signer = ECDSA.recover(
                sigHash,
                uint8(signature.v),
                signature.r,
                signature.s
            );
            require(
                validatorSet.validators[signer] == true,
                "RollupV1: Signer is not a validator"
            );

            require(signer > previous, "RollupV1: Signers are not sorted");
            previous = signer;
        }
    }

    function getSignatureMessageHash(
        bytes32 newRoot,
        uint256 height,
        bytes32 otherHashFromBlockHash
    ) internal pure returns (bytes32) {
        bytes32 proposalHash = keccak256(
            abi.encode(newRoot, height, otherHashFromBlockHash)
        );
        bytes32 acceptMsg = keccak256(abi.encode(height + 1, proposalHash));
        bytes32 sigMsg = keccak256(
            abi.encodePacked(NETWORK_LEN, NETWORK, acceptMsg)
        );
        return sigMsg;
    }

    // Returns all validator sets from a given index, inclusive
    function getValidatorSets(
        uint256 from
    ) public view returns (PublicValidatorSet[] memory) {
        PublicValidatorSet[] memory sets = new PublicValidatorSet[](
            validatorSets.length - from
        );

        for (uint256 i = from; i < validatorSets.length; i++) {
            sets[i - from] = PublicValidatorSet(
                validatorSets[i].validatorsArray,
                validatorSets[i].validFrom
            );
        }

        return sets;
    }

    function getValidators() internal view returns (ValidatorSet storage) {
        return validatorSets[validatorSetIndex];
    }

    function _setValidators(
        uint256 validFrom,
        address[] calldata validators
    ) private {
        require(
            validatorSets.length == 0 ||
            validatorSets[validatorSets.length - 1].validFrom < validFrom,
            "RollupV1: New validator set must have a validFrom greater than the last set"
        );
        require(
            validFrom == 0 || validFrom <= block.number + MAX_FUTURE_BLOCKS,
            "RollupV1: validFrom cannot be more than 30 days in the future"
        );

        // V2 (Idea 4): enforce a minimum activation notice window for
        // every non-initial validator set.
        //
        // The initial validator set (when validatorSets.length == 0)
        // skips this check — deployment naturally establishes it with
        // validFrom=0, and there are no users yet to protect.
        //
        // For every subsequent rotation, the floor guarantees users
        // get `validatorActivationMinDelayBlocks` of notice regardless
        // of what delay the governance layer (timelock) is using.
        // This is defense-in-depth: even if governance is subverted
        // or its delay is shortened, the contract itself still
        // enforces a minimum reaction time.
        if (validatorSets.length > 0) {
            require(
                validFrom >= block.number + validatorActivationMinDelayBlocks,
                "RollupV1: validator activation too early"
            );
        }

        // Create a new ValidatorSet and push it to the array
        validatorSets.push();
        uint256 newIndex = validatorSets.length - 1;

        validatorSets[newIndex].validFrom = validFrom;
        validatorSets[newIndex].validatorsArray = validators;

        for (uint256 i = 0; i < validators.length; i++) {
            require(
                validatorSets[newIndex].validators[validators[i]] == false,
                "RollupV1: Validator already exists"
            );

            validatorSets[newIndex].validators[validators[i]] = true;
        }

        emit ValidatorSetAdded(newIndex, validFrom);
    }

    function setValidators(
        uint256 validFrom,
        address[] calldata validators
    ) public onlyOwner {
        _setValidators(validFrom, validators);
    }

    /**
     * @notice Update the EscrowManager address (onlyOwner)
     * @param newEscrowManagerAddress The new EscrowManager address
     */
    function setEscrowManager(
        address newEscrowManagerAddress
    ) external onlyOwner {
        require(newEscrowManagerAddress != address(0), "RollupV1: Invalid escrow manager address");
        emit EscrowManagerUpdated(escrowManager, newEscrowManagerAddress);
        escrowManager = newEscrowManagerAddress;
    }

    function updateValidatorSetIndex(uint256 height) internal {
        for (uint256 i = validatorSetIndex + 1; i < validatorSets.length; i++) {
            if (validatorSets[i].validFrom > height) {
                break;
            }

            validatorSetIndex = i;
        }
    }

    // Debug function to expose internal hash calculation
    function debugGetSignatureMessageHash(
        bytes32 newRoot,
        uint256 height,
        bytes32 otherHashFromBlockHash
    ) external pure returns (bytes32) {
        return getSignatureMessageHash(newRoot, height, otherHashFromBlockHash);
    }

    // Debug function to expose intermediate values
    function debugGetIntermediateHashes(
        bytes32 newRoot,
        uint256 height,
        bytes32 otherHashFromBlockHash
    )
    external
    pure
    returns (bytes32 proposalHash, bytes32 acceptMsg, bytes32 sigMsg)
    {
        proposalHash = keccak256(
            abi.encode(newRoot, height, otherHashFromBlockHash)
        );
        acceptMsg = keccak256(abi.encode(height + 1, proposalHash));
        sigMsg = keccak256(abi.encodePacked(NETWORK_LEN, NETWORK, acceptMsg));
    }

    // Debug function to see the raw packed bytes
    function debugGetPackedBytes(
        bytes32 acceptMsg
    ) external pure returns (bytes memory) {
        return abi.encodePacked(NETWORK_LEN, NETWORK, acceptMsg);
    }
}
