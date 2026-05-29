// SetValidatorsViaTimelock.test.ts
//
// After initialize(), the rollup's owner is the in-contract timelock.
// Confirms that scripts/set-validators.ts's pattern works end-to-end:
// schedule → wait min-delay → execute. Also confirms that a direct
// setValidators() call from an EOA reverts.

import { describe, it } from "node:test";
import assert from "node:assert/strict";
import hre from "hardhat";
import {
  encodeAbiParameters,
  encodeFunctionData,
  keccak256,
  parseEther,
  zeroAddress,
  zeroHash,
} from "viem";

const ONE_HOUR = 3_600n;
const SEVEN_DAYS = 7n * 24n * ONE_HOUR;
const SATS_TO_WEI = 10_000_000_000n;
const VK_HASH =
  "0x0070170dcc7ad428aceab8724ef8ea429eb8baa18a1dc7f687d5ec3536f11fcc" as const;

function deriveNoteKind(chainId: number, tokenAddress: string): `0x${string}` {
  const prefix = "0002000000000000" + chainId.toString(16).padStart(4, "0");
  const tokenHex = tokenAddress.slice(2).toLowerCase().padStart(40, "0");
  return ("0x" + prefix + tokenHex + "0000").toLowerCase() as `0x${string}`;
}

const TIMELOCK_ABI = [
  {
    type: "function",
    name: "getMinDelay",
    inputs: [],
    outputs: [{ type: "uint256" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "hashOperation",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
    ],
    outputs: [{ type: "bytes32" }],
    stateMutability: "pure",
  },
  {
    type: "function",
    name: "schedule",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
      { name: "delay", type: "uint256" },
    ],
    outputs: [],
    stateMutability: "nonpayable",
  },
  {
    type: "function",
    name: "execute",
    inputs: [
      { name: "target", type: "address" },
      { name: "value", type: "uint256" },
      { name: "data", type: "bytes" },
      { name: "predecessor", type: "bytes32" },
      { name: "salt", type: "bytes32" },
    ],
    outputs: [],
    stateMutability: "payable",
  },
  {
    type: "function",
    name: "isOperationReady",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "bool" }],
    stateMutability: "view",
  },
  {
    type: "function",
    name: "isOperationDone",
    inputs: [{ name: "id", type: "bytes32" }],
    outputs: [{ type: "bool" }],
    stateMutability: "view",
  },
] as const;

async function deployRollup(viem: any, publicClient: any) {
  const [owner, prover, validator, sink, burner] =
    await viem.getWalletClients();
  const token = await viem.deployContract("MockERC20");
  const verifier = await viem.deployContract("MockVerifier");
  const impl = await viem.deployContract("RollupV1");

  const chainId = await publicClient.getChainId();
  const noteKind = deriveNoteKind(chainId, token.address);

  const initData = encodeFunctionData({
    abi: impl.abi,
    functionName: "initialize",
    args: [
      owner.account.address,
      burner.account.address,
      token.address,
      noteKind,
      verifier.address,
      prover.account.address,
      [validator.account.address],
      VK_HASH,
      parseEther("0.001"),
      parseEther("10"),
      SEVEN_DAYS,
      300n * SATS_TO_WEI,
      sink.account.address,
      ONE_HOUR,
      [owner.account.address],
      [zeroAddress],
    ],
  });

  const proxy = await viem.deployContract("RollupTestProxy", [
    impl.address,
    owner.account.address,
    initData,
  ]);

  const rollup = await viem.getContractAt("RollupV1", proxy.address);
  return { rollup, owner };
}

describe("setValidators via timelock", () => {
  it("rejects direct setValidators() from an EOA", async () => {
    const { viem } = await hre.network.connect();
    const publicClient = await viem.getPublicClient();
    const { rollup } = await deployRollup(viem, publicClient);

    await assert.rejects(async () => {
      await rollup.write.setValidators([
        1n,
        ["0x1111111111111111111111111111111111111111"],
      ]);
    });
  });

  it("succeeds when scheduled and executed through the timelock", async () => {
    const { viem, networkHelpers } = await hre.network.connect();
    const publicClient = await viem.getPublicClient();
    const { rollup, owner } = await deployRollup(viem, publicClient);

    const timelockAddr = (await rollup.read.timelock()) as `0x${string}`;

    const newValidators = [
      "0x1111111111111111111111111111111111111111",
      "0x2222222222222222222222222222222222222222",
    ] as const;
    const data = encodeFunctionData({
      abi: rollup.abi,
      functionName: "setValidators",
      args: [1n, newValidators as any],
    });
    const target = rollup.address;
    const salt = keccak256(
      encodeAbiParameters(
        [{ type: "address" }, { type: "bytes" }],
        [target, data],
      ),
    );

    const opId = (await publicClient.readContract({
      address: timelockAddr,
      abi: TIMELOCK_ABI,
      functionName: "hashOperation",
      args: [target, 0n, data, zeroHash, salt],
    })) as `0x${string}`;

    assert.equal(
      await publicClient.readContract({
        address: timelockAddr,
        abi: TIMELOCK_ABI,
        functionName: "isOperationDone",
        args: [opId],
      }),
      false,
    );

    const delay = (await publicClient.readContract({
      address: timelockAddr,
      abi: TIMELOCK_ABI,
      functionName: "getMinDelay",
    })) as bigint;

    const scheduleHash = await owner.writeContract({
      address: timelockAddr,
      abi: TIMELOCK_ABI,
      functionName: "schedule",
      args: [target, 0n, data, zeroHash, salt, delay],
    });
    await publicClient.waitForTransactionReceipt({ hash: scheduleHash });

    // Time-warp past timelock delay.
    await networkHelpers.time.increase(Number(delay) + 1);

    assert.equal(
      await publicClient.readContract({
        address: timelockAddr,
        abi: TIMELOCK_ABI,
        functionName: "isOperationReady",
        args: [opId],
      }),
      true,
    );

    const execHash = await owner.writeContract({
      address: timelockAddr,
      abi: TIMELOCK_ABI,
      functionName: "execute",
      args: [target, 0n, data, zeroHash, salt],
    });
    await publicClient.waitForTransactionReceipt({ hash: execHash });

    assert.equal(
      await publicClient.readContract({
        address: timelockAddr,
        abi: TIMELOCK_ABI,
        functionName: "isOperationDone",
        args: [opId],
      }),
      true,
    );

    const sets = (await rollup.read.getValidatorSets([0n])) as Array<{
      validators: readonly string[];
    }>;
    const latest = sets[sets.length - 1];
    assert.deepEqual(
      latest.validators.map((v) => v.toLowerCase()),
      newValidators.map((v) => v.toLowerCase()),
    );
  });
});
