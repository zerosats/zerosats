// RenounceGuard.test.ts
//
// Contract-level proof that the rollup's renounce path goes through the
// timelock: a direct EOA renounceOwnership() reverts, the timelock-dispatched
// one succeeds after the configured delay.

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

describe("Renounce guard", () => {
  it("rejects renounceOwnership() from the previous deployer EOA", async () => {
    const { viem } = await hre.network.connect();
    const publicClient = await viem.getPublicClient();
    const { rollup } = await deployRollup(viem, publicClient);
    await assert.rejects(async () => {
      await rollup.write.renounceOwnership();
    });
  });

  it("allows the timelock to renounceOwnership after the delay", async () => {
    const { viem, networkHelpers } = await hre.network.connect();
    const publicClient = await viem.getPublicClient();
    const { rollup, owner } = await deployRollup(viem, publicClient);
    const timelockAddr = (await rollup.read.timelock()) as `0x${string}`;

    const data = encodeFunctionData({
      abi: rollup.abi,
      functionName: "renounceOwnership",
      args: [],
    });
    const target = rollup.address;
    const salt = keccak256(
      encodeAbiParameters(
        [{ type: "address" }, { type: "bytes" }],
        [target, data],
      ),
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

    await networkHelpers.time.increase(Number(delay) + 1);

    const execHash = await owner.writeContract({
      address: timelockAddr,
      abi: TIMELOCK_ABI,
      functionName: "execute",
      args: [target, 0n, data, zeroHash, salt],
    });
    await publicClient.waitForTransactionReceipt({ hash: execHash });

    assert.equal(await rollup.read.owner(), zeroAddress);
  });
});
