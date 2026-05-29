// Convenience: print a fresh BIP39 mnemonic + the first derived account.
// Suitable for development and ad-hoc testing ONLY. Do not use for mainnet
// signers — generate mainnet keys via a hardware wallet or vetted offline tool.

import { english, generateMnemonic, mnemonicToAccount } from "viem/accounts";
import { toHex } from "viem";

async function main() {
  const mnemonic = generateMnemonic(english);
  const account = mnemonicToAccount(mnemonic);
  const privateKey = toHex(account.getHdKey().privateKey!);

  console.log("⚠️  Generated key material is suitable for DEV USE ONLY.");
  console.log("⚠️  Do not use this output as a mainnet signer.\n");

  console.log("Generated Random Account:");
  console.log("-------------------------");
  console.log(`Mnemonic:    ${mnemonic}`);
  console.log(`Address:     ${account.address}`);
  console.log(`Private Key: ${privateKey}`);
  console.log(`Public Key:  ${account.publicKey}`);
}

main().catch(console.error);
