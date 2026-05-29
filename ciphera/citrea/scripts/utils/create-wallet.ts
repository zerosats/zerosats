import { generatePrivateKey, privateKeyToAccount } from 'viem/accounts';

// 1. Generate a cryptographically secure random private key (hex string with '0x')
const privateKey = generatePrivateKey();

// 2. Create an account instance from the private key
const account = privateKeyToAccount(privateKey);

// 3. Extract the address and private key
const address = account.address;
// The privateKey variable from generatePrivateKey() is already the hex private key

console.log('--- Generated Wallet ---');
console.log('Address:', address);
console.log('Private Key (Hex):', privateKey);

// Optional: If you need the raw hex without the '0x' prefix
const privateKeyRaw = privateKey.slice(2);
console.log('Private Key (Raw Hex):', privateKeyRaw);