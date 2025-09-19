import { HardhatUserConfig } from "hardhat/config";

import hardhatToolboxViemPlugin from "@nomicfoundation/hardhat-toolbox-viem";
import { configVariable } from "hardhat/config";

const config: HardhatUserConfig = {
  plugins: [hardhatToolboxViemPlugin],
  solidity: {
    version: '0.8.22',
    settings: {
      viaIR: true,
      optimizer: {
        enabled: true,
        runs: 200
      }
    }
  },
  networks: {
    hardhatMainnet: {
      type: "edr-simulated",
      chainType: "l1",
    },
    hardhatOp: {
      type: "edr-simulated",
      chainType: "op",
    },
    sepolia: {
      type: "http",
      chainType: "l1",
      url: `https://eth-sepolia.g.alchemy.com/v2/${configVariable("ALCHEMY_API_KEY")}`,
      accounts: { mnemonic:configVariable("MNEMONIC")},
    },
    citreaTestnet: {
      type: "http",
      chainId: 5115,
      url: "https://rpc.testnet.citrea.xyz",
      accounts: { mnemonic:configVariable("MNEMONIC")},
    },
    citreaDevnet: {
      type: "http",
      chainId: 5655,
      url: "http://localhost:12345",
      accounts: { mnemonic:configVariable("MNEMONIC")},
    },
  },
};

export default config;
