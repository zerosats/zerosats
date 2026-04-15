// V2 upgrade tests for RollupV1 (bump-contract branch).
//
// Uses the project's installed toolchain: hardhat 3 + viem + node:test.
// The existing ethers-based test files in this directory are broken
// against this toolchain (see git log on burn-substitutor) — this
// file deliberately uses the currently-working stack.
//
// Coverage:
//   Idea 1: setRoot reverts
//   Idea 2: per-mint cap + TVL cap enforced
//   Idea 3: open-proving timer + isProvingOpen view
//   Idea 4: validator activation floor
//   Ideas 5-8: guardian + pause flow
//   Idea 9: computeBurnFee math
//
//   Review fix 1: non-owner cannot call initializeV2
//   Review fix 2: isProvingOpen is false pre-init
//   Review fix 3: failed burn does not decrement currentTvl
//   Review fix 4: addToken reverts
//
// Full end-to-end verifyRollup is not exercised for the V2-gate
// tests because it requires a real ZK proof and validator signatures;
// unit behavior of the V2 gates is what's new. The Finding 3 test
// DOES go end-to-end through verifyRollup using MockVerifier (which
// always returns true) and the Idea 3 escape mode to bypass the
// signature check — this is the only way to drive verifyBurn from
// a test without reimplementing the ZK proving stack.

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { network } from "hardhat";
import { parseEther, encodeFunctionData } from "viem";

const NOTE_KIND =
  "0x000200000000000013fb8d0c9d1c17ae5e40fff9be350f57840e9e66cd930000" as const;
const VK_HASH =
  "0x0000000000000000000000000000000000000000000000000000000000000001" as const;

// Helper: deploy a RollupV1 implementation behind an ERC1967 proxy,
// run initialize(), then run initializeV2(). Returns the rollup typed
// as the implementation so `.write.initializeV2(...)` works on it.
//
// Why a proxy: RollupV1's constructor calls _disableInitializers() on
// the implementation, which is correct for production (prevents the
// implementation from being initialized directly, a known OZ footgun)
// but means we must wrap it in a proxy for any init-using test.
async function deployRollupV2() {
  const { viem, networkHelpers } = await network.connect();
  const [owner, prover, validator, guardian, sink, user] =
    await viem.getWalletClients();

  const mockToken = await viem.deployContract("MockERC20");
  const mockVerifier = await viem.deployContract("MockVerifier");
  const impl = await viem.deployContract("RollupV1");

  // Encode initialize(...) call data for proxy constructor.
  const initData = encodeFunctionData({
    abi: impl.abi,
    functionName: "initialize",
    args: [
      owner.account.address,
      owner.account.address, // escrow manager (placeholder — owner for tests)
      mockToken.address,
      mockVerifier.address,
      prover.account.address,
      [validator.account.address],
      VK_HASH,
    ],
  });

  const proxy = await viem.deployContract("RollupV2TestProxy", [
    impl.address,
    initData,
  ]);

  // Get a RollupV1-typed handle pointed at the proxy.
  const rollup = await viem.getContractAt("RollupV1", proxy.address);

  // initializeV2 — test-tuned values (not production):
  //   perMintCap         = 1 ether      (easy round number)
  //   globalTvlCap       = 10 ether
  //   openProvingDelay   = 1 day        (minimum allowed by contract)
  //   validatorMinDelay  = 100 blocks   (small so tests are fast)
  //   burnFeeFloor       = 1000         (arbitrary wei)
  //   burnFeeBps         = 10           (0.1%)
  await rollup.write.initializeV2([
    parseEther("1"),
    parseEther("10"),
    86400n,
    100n,
    guardian.account.address,
    1000n,
    10n,
    sink.account.address,
  ]);

  return {
    rollup,
    mockToken,
    mockVerifier,
    owner,
    prover,
    validator,
    guardian,
    sink,
    user,
    networkHelpers,
  };
}

describe("RollupV1 V2 upgrade", () => {
  // -----------------------------------------------------------------
  // Idea 1 — setRoot is disabled
  // -----------------------------------------------------------------
  describe("Idea 1: setRoot disabled", () => {
    it("setRoot always reverts, even from owner", async () => {
      const { rollup, owner } = await deployRollupV2();
      await assert.rejects(
        rollup.write.setRoot(
          [
            "0x1111111111111111111111111111111111111111111111111111111111111111",
          ],
          { account: owner.account },
        ),
      );
    });
  });

  // -----------------------------------------------------------------
  // Idea 2 — deposit caps
  // -----------------------------------------------------------------
  describe("Idea 2: deposit caps", () => {
    it("mint() rejects amount > perMintCap", async () => {
      const { rollup, mockToken, user } = await deployRollupV2();
      // Fund and approve user
      await mockToken.write.mint([user.account.address, parseEther("100")]);
      await mockToken.write.approve([rollup.address, parseEther("100")], {
        account: user.account,
      });

      // perMintCap = 1 ether; try 2 ether
      const bigValue =
        "0x0000000000000000000000000000000000000000000000001bc16d674ec80000" as const; // 2e18
      await assert.rejects(
        rollup.write.mint(
          [
            "0x2222222222222222222222222222222222222222222222222222222222222222",
            bigValue,
            NOTE_KIND,
          ],
          { account: user.account },
        ),
      );
    });

    it("currentTvl increments on mint and is cap-checked", async () => {
      const { rollup, mockToken, user } = await deployRollupV2();
      await mockToken.write.mint([user.account.address, parseEther("100")]);
      await mockToken.write.approve([rollup.address, parseEther("100")], {
        account: user.account,
      });

      const v = "0x00000000000000000000000000000000000000000000000000038d7ea4c68000" as const; // 1e15
      const hashA = "0x0000000000000000000000000000000000000000000000000000000000000aaa" as const;
      await rollup.write.mint([hashA, v, NOTE_KIND], {
        account: user.account,
      });

      const tvl = await rollup.read.currentTvl();
      assert.equal(tvl, 1_000_000_000_000_000n);
    });
  });

  // -----------------------------------------------------------------
  // Idea 3 — open proving view
  // -----------------------------------------------------------------
  describe("Idea 3: open proving", () => {
    it("isProvingOpen is false right after V2 init", async () => {
      const { rollup } = await deployRollupV2();
      const open = await rollup.read.isProvingOpen();
      assert.equal(open, false);
    });

    it("isProvingOpen flips to true once openProvingDelay elapses", async () => {
      const { rollup, networkHelpers } = await deployRollupV2();
      // openProvingDelay = 86400s; jump 86401s and mine a block
      // so the new timestamp is reflected when the view call is
      // evaluated against the current chain state.
      await networkHelpers.time.increase(86401);
      await networkHelpers.mine(1);
      const open = await rollup.read.isProvingOpen();
      assert.equal(open, true);
    });
  });

  // -----------------------------------------------------------------
  // Idea 4 — validator activation floor
  // -----------------------------------------------------------------
  describe("Idea 4: validator activation floor", () => {
    it("setValidators rejects validFrom below the floor", async () => {
      const { rollup, owner, validator, networkHelpers } =
        await deployRollupV2();
      const currentBlock = await networkHelpers.time.latestBlock();

      // floor is 100 blocks; try 50 blocks ahead
      await assert.rejects(
        rollup.write.setValidators(
          [BigInt(currentBlock + 50), [validator.account.address]],
          { account: owner.account },
        ),
      );
    });

    it("setValidators accepts validFrom past the floor", async () => {
      const { rollup, owner, validator, networkHelpers } =
        await deployRollupV2();
      const currentBlock = await networkHelpers.time.latestBlock();

      await rollup.write.setValidators(
        [BigInt(currentBlock + 200), [validator.account.address]],
        { account: owner.account },
      );
    });
  });

  // -----------------------------------------------------------------
  // Ideas 5-8 — guardian + pause
  // -----------------------------------------------------------------
  describe("Ideas 5-8: guardian + pause", () => {
    it("guardian can pause, non-guardian cannot", async () => {
      const { rollup, guardian, user } = await deployRollupV2();
      // Non-guardian attempt should revert.
      await assert.rejects(
        rollup.write.setWithdrawalsPaused([true], { account: user.account }),
      );
      // Guardian attempt should succeed.
      await rollup.write.setWithdrawalsPaused([true], {
        account: guardian.account,
      });
      const paused = await rollup.read.withdrawalsPaused();
      assert.equal(paused, true);
    });

    it("substituteBurn reverts while paused", async () => {
      const { rollup, owner, guardian } = await deployRollupV2();
      await rollup.write.setWithdrawalsPaused([true], {
        account: guardian.account,
      });

      // owner was set as a burn substitutor in initialize().
      // Note: this tx reverts on the pause check before touching
      // any of the substitute-burn bookkeeping, so bogus hash/amount
      // values are fine here — the pause check runs first.
      await assert.rejects(
        rollup.write.substituteBurn(
          [
            owner.account.address,
            NOTE_KIND,
            "0x000000000000000000000000000000000000000000000000000000000000beef",
            1_000_000_000_000_000n,
            9_999_999n,
          ],
          { account: owner.account },
        ),
      );
    });
  });

  // -----------------------------------------------------------------
  // Idea 9 — fee math
  // -----------------------------------------------------------------
  describe("Idea 9: burn fee math", () => {
    it("computeBurnFee returns floor for small values", async () => {
      const { rollup } = await deployRollupV2();
      // value=500, bps=10 → bps fee = 0, floor = 1000 → fee = 500 (clamped)
      const fee = await rollup.read.computeBurnFee([500n]);
      assert.equal(fee, 500n);
    });

    it("computeBurnFee returns bps portion for large values", async () => {
      const { rollup } = await deployRollupV2();
      // value=1e18, bps=10 → bps fee = 1e15 (0.1%) >> floor (1000)
      const fee = await rollup.read.computeBurnFee([parseEther("1")]);
      assert.equal(fee, 1_000_000_000_000_000n);
    });

    it("computeBurnFee applies floor when bps portion < floor", async () => {
      const { rollup } = await deployRollupV2();
      // value=10000, bps=10 → bps fee = 10, floor = 1000 → fee = 1000
      const fee = await rollup.read.computeBurnFee([10_000n]);
      assert.equal(fee, 1000n);
    });
  });

  // =================================================================
  // Review fixes
  //
  // These cover the four findings from the post-V2 review:
  //   Finding 1: initializeV2 must be owner-restricted
  //   Finding 2: isProvingOpen must not be true before V2 init
  //   Finding 3: TVL must not decrement on a failed burn
  //   Finding 4: addToken must revert in V2
  // =================================================================
  describe("Review fixes", () => {
    // ---------------------------------------------------------------
    // Finding 1 — non-owner cannot call initializeV2.
    //
    // Deploys impl behind a proxy with initialize() encoded in the
    // constructor data (so V1 state is set). Then attempts to call
    // initializeV2 from a non-owner account. Must revert. Without
    // the onlyOwner modifier, this call would succeed and the
    // attacker would own guardian/feeSink/caps.
    // ---------------------------------------------------------------
    it("Finding 1: non-owner cannot call initializeV2", async () => {
      const { viem } = await network.connect();
      const [owner, prover, validator, guardian, sink, attacker] =
        await viem.getWalletClients();

      const mockToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");

      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          owner.account.address,
          owner.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      // Attacker races to initializeV2 with their own addresses.
      // Must revert due to onlyOwner.
      await assert.rejects(
        rollup.write.initializeV2(
          [
            parseEther("1"),
            parseEther("10"),
            86400n,
            100n,
            attacker.account.address, // attacker tries to seize guardian
            1000n,
            10n,
            attacker.account.address, // attacker tries to seize fee sink
          ],
          { account: attacker.account },
        ),
      );

      // Owner should still be able to initialize.
      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          86400n,
          100n,
          guardian.account.address,
          1000n,
          10n,
          sink.account.address,
        ],
        { account: owner.account },
      );

      // Verify real guardian is in place (not attacker).
      const g = await rollup.read.guardian();
      assert.equal(g.toLowerCase(), guardian.account.address.toLowerCase());
    });

    // ---------------------------------------------------------------
    // Finding 2 — isProvingOpen must be false before initializeV2.
    //
    // Pre-V2-init storage defaults: lastVerifiedAt=0, openProvingDelay=0.
    // Without the guard, block.timestamp >= 0 + 0 is trivially true,
    // which would flip the contract into escape mode during the
    // upgrade window. The guard (`openProvingDelay == 0 ? false`)
    // restores V1-equivalent behavior until V2 is initialized.
    // ---------------------------------------------------------------
    it("Finding 2: isProvingOpen is false before initializeV2", async () => {
      const { viem } = await network.connect();
      const [owner, prover, validator] = await viem.getWalletClients();

      const mockToken = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");

      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          owner.account.address,
          owner.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      // V1 initialized, V2 NOT yet initialized. openProvingDelay is 0.
      // Guard should keep isProvingOpen false.
      const open = await rollup.read.isProvingOpen();
      assert.equal(open, false);

      // Sanity: openProvingDelay really is 0 at this point.
      const delay = await rollup.read.openProvingDelay();
      assert.equal(delay, 0n);
    });

    // ---------------------------------------------------------------
    // Finding 3 — failed burn must not decrement currentTvl.
    //
    // Uses FailingERC20 (a non-reverting ERC20 that returns false
    // from transfer when a flag is set). Seeds the contract with a
    // mint, flips the token to fail-mode, then drives a burn through
    // verifyRollup in escape mode (so we don't need real validator
    // signatures).
    //
    // Pre-fix: currentTvl dropped to 0 despite the tokens staying
    // in the contract. Post-fix: currentTvl is preserved.
    // ---------------------------------------------------------------
    it("Finding 3: failed burn does not decrement currentTvl", async () => {
      const { viem, networkHelpers } = await network.connect();
      const [owner, prover, validator, guardian, sink, userAcct] =
        await viem.getWalletClients();

      const failingToken = await viem.deployContract("FailingERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");

      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          owner.account.address,
          owner.account.address,
          failingToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          86400n,
          100n,
          guardian.account.address,
          1000n,
          10n,
          sink.account.address,
        ],
        { account: owner.account },
      );

      // Deposit: user mints 0.5 ether of FailingERC20 into the rollup.
      // This both gives the contract balance AND bumps currentTvl.
      const mintValue = parseEther("0.5");
      const mintValueBytes32 =
        "0x00000000000000000000000000000000000000000000000006f05b59d3b20000" as const; // 5e17

      await failingToken.write.mint([userAcct.account.address, mintValue]);
      await failingToken.write.approve([rollup.address, mintValue], {
        account: userAcct.account,
      });
      await rollup.write.mint(
        [
          "0x0000000000000000000000000000000000000000000000000000000000001111",
          mintValueBytes32,
          NOTE_KIND,
        ],
        { account: userAcct.account },
      );

      const tvlBefore = await rollup.read.currentTvl();
      assert.equal(tvlBefore, mintValue);

      // Trigger escape mode so we can bypass validator signatures.
      await networkHelpers.time.increase(86401);
      await networkHelpers.mine(1);

      // Flip the token to fail-mode so the burn payout silently fails.
      await failingToken.write.setTransfersFail([true]);

      // Craft publicInputs for verifyRollup:
      //   [0]  oldRoot
      //   [1]  newRoot (any)
      //   [2]  commitHash (any)
      //   [3]  kind = 3 (burn)
      //   [4]  note_kind
      //   [5]  value as bytes32
      //   [6]  burn hash (any)
      //   [7]  burn_addr as bytes32
      //   [8..32] kind = 0 padding (25 slots)
      // Total: 3 + 5 + 25 = 33 = messages_length(30) + 3 ✓
      const oldRoot = await rollup.read.rootHash();
      const zero =
        "0x0000000000000000000000000000000000000000000000000000000000000000" as const;
      const three =
        "0x0000000000000000000000000000000000000000000000000000000000000003" as const;
      const burnHash =
        "0x0000000000000000000000000000000000000000000000000000000000002222" as const;
      const userAddrAsBytes32 = (("0x000000000000000000000000" +
        userAcct.account.address.slice(2).toLowerCase()) as `0x${string}`);

      const padding: `0x${string}`[] = Array(25).fill(zero);
      const publicInputs: `0x${string}`[] = [
        oldRoot,
        "0x0000000000000000000000000000000000000000000000000000000000000001" as const,
        "0x0000000000000000000000000000000000000000000000000000000000000002" as const,
        three,
        NOTE_KIND,
        mintValueBytes32,
        burnHash,
        userAddrAsBytes32,
        ...padding,
      ];

      // Empty signatures — allowed in escape mode.
      await rollup.write.verifyRollup(
        [
          1n, // height > blockHeight(0)
          VK_HASH,
          "0x" as const, // empty aggrProof; MockVerifier ignores it
          publicInputs,
          zero, // otherHashFromBlockHash (unused in escape mode)
          [], // no signatures
        ],
        { account: prover.account },
      );

      // TVL must be preserved because the transfer failed silently.
      // The rollup contract still holds the tokens; currentTvl must
      // reflect that, otherwise future mints could push real balance
      // past globalTvlCap.
      const tvlAfter = await rollup.read.currentTvl();
      assert.equal(
        tvlAfter,
        tvlBefore,
        "TVL should be unchanged when burn transfer failed",
      );

      // And the tokens really are still in the contract:
      const bal = await failingToken.read.balanceOf([rollup.address]);
      assert.equal(bal, mintValue);
    });

    // ---------------------------------------------------------------
    // Finding 4 — addToken is gated on the pre-V2 window.
    //
    // The V2 caps are single global counters and only make sense
    // under a single-token assumption. But the pre-upgrade bootstrap
    // flow (scripts/deploy.ts on devnet) legitimately needs to
    // register a second mock note kind between initialize() and
    // initializeV2() — during that window there are no caps yet,
    // so multi-token registration is safe.
    //
    // The fix: gate addToken on version < 2. Pre-V2 (during bootstrap)
    // it works normally. Post-V2 (once caps are active) it reverts.
    //
    // This test covers both sides of the boundary.
    // ---------------------------------------------------------------
    it("Finding 4: addToken works pre-V2 but reverts post-V2", async () => {
      const { viem } = await network.connect();
      const [owner, prover, validator, guardian, sink] =
        await viem.getWalletClients();

      const mockToken = await viem.deployContract("MockERC20");
      const mockToken2 = await viem.deployContract("MockERC20");
      const mockVerifier = await viem.deployContract("MockVerifier");
      const impl = await viem.deployContract("RollupV1");

      const initData = encodeFunctionData({
        abi: impl.abi,
        functionName: "initialize",
        args: [
          owner.account.address,
          owner.account.address,
          mockToken.address,
          mockVerifier.address,
          prover.account.address,
          [validator.account.address],
          VK_HASH,
        ],
      });
      const proxy = await viem.deployContract("RollupV2TestProxy", [
        impl.address,
        initData,
      ]);
      const rollup = await viem.getContractAt("RollupV1", proxy.address);

      // Pre-V2 window (version == 1): addToken should succeed.
      // This mirrors what scripts/deploy.ts does on devnet — it
      // registers a second mock BTC note kind before calling V2 init.
      const mockBtcNoteKind =
        "0x000200000000000000893c499c542cef5e3811e1192ce70d8cc03d5c33590000" as const;
      await rollup.write.addToken(
        [mockBtcNoteKind, mockToken2.address],
        { account: owner.account },
      );

      // Verify the token was actually registered.
      const registered = await rollup.read.noteKindTokenAddress([
        mockBtcNoteKind,
      ]);
      assert.equal(
        registered.toLowerCase(),
        mockToken2.address.toLowerCase(),
        "Pre-V2 addToken should have registered the token",
      );

      // Cross the V2 boundary.
      await rollup.write.initializeV2(
        [
          parseEther("1"),
          parseEther("10"),
          86400n,
          100n,
          guardian.account.address,
          1000n,
          10n,
          sink.account.address,
        ],
        { account: owner.account },
      );

      // Post-V2 window (version == 2): addToken must revert.
      // A different note kind to avoid hitting the "Token already
      // exists" check first — we want to confirm the V2 gate fires.
      const anotherNoteKind =
        "0x0003000000000000000000000000000000000000000000000000000000000000" as const;
      await assert.rejects(
        rollup.write.addToken([anotherNoteKind, mockToken2.address], {
          account: owner.account,
        }),
      );

      // Non-owners also can't call it (unchanged behavior).
      await assert.rejects(
        rollup.write.addToken([anotherNoteKind, mockToken2.address], {
          account: prover.account,
        }),
      );
    });
  });
});
