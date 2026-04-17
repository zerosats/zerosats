import { HardhatUserConfig } from "hardhat/config";

import hardhatToolboxViemPlugin from "@nomicfoundation/hardhat-toolbox-viem";
import { configVariable } from "hardhat/config";

const config: HardhatUserConfig = {
  plugins: [hardhatToolboxViemPlugin],

  solidity: {
    compilers: [
      {
        version: "0.8.28",
        settings: {
          viaIR: true,
          optimizer: { enabled: true, runs: 200 },
        },
      },
    ],
  },
  networks: {
    hardhatMainnet: {
      type: "edr-simulated",
      chainType: "l1",
    },
    citreaDevnet: {
      type: "http",
      chainId: 5655,
      url: "http://localhost:12345",
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
    citreaTestnet: {
      type: "http",
      chainId: 5115,
      url: "https://rpc.testnet.citrea.xyz",
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
    citreaMainnet: {
      type: "http",
      chainId: 4114,
      url:
          `https://citrea-mainnet.g.alchemy.com/v2/${configVariable("ALCHEMY_MAINNET_API_KEY")}`,
      accounts: { mnemonic: configVariable("MNEMONIC") },
    },
  },

  chainDescriptors: {
    5115: {
      name: "citreaTestnet",
      blockExplorers: {
        blockscout: {
          name: "Citrea Testnet Explorer",
          url: "https://explorer.testnet.citrea.xyz",
          apiUrl: "https://explorer.testnet.citrea.xyz/api",
        },
      },
    },
    4114: {
      name: "citreaMainnet",
      blockExplorers: {
        blockscout: {
          name: "Citrea Mainnet Explorer",
          url: "https://explorer.mainnet.citrea.xyz",
          apiUrl: "https://explorer.mainnet.citrea.xyz/api",
        },
      },
    },
  },
};

export default config;