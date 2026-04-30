use ethereum_types::{Address, H256, U256};

/// Flags used in EscrowData
const FLAG_PAY_OUT: u64 = 0x01;
const FLAG_PAY_IN: u64 = 0x02;
const FLAG_REPUTATION: u64 = 0x04;

/// Represents the decoded flags from the uint256 flags field
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flags {
    pub pay_out: bool,
    pub pay_in: bool,
    pub reputation: bool,
    /// Upper 64 bits used as sequence number
    pub sequence: u64,
}

impl Flags {
    /// Decode flags from uint256
    /// Sequence is in upper 64 bits (after right shift by 64)
    pub fn from_u256(value: U256) -> Self {
        // Extract upper 64 bits (bits 64-127)
        let sequence = (value >> 64u32).as_u64();

        // Extract lower 64 bits by masking (bits 0-63)
        let mask = U256::from(0xFFFFFFFFFFFFFFFFu64);
        let lower_bits = (value & mask).as_u64();

        Flags {
            sequence,
            pay_out: (lower_bits & FLAG_PAY_OUT) == FLAG_PAY_OUT,
            pay_in: (lower_bits & FLAG_PAY_IN) == FLAG_PAY_IN,
            reputation: (lower_bits & FLAG_REPUTATION) == FLAG_REPUTATION,
        }
    }
    /// Encode flags to U256
    /// Matches TypeScript: (sequence << 64n) | flags
    pub fn to_u256(&self) -> U256 {
        let sequence_bits = U256::from(self.sequence) << 64u32;
        let flag_bits = U256::from(
            (if self.pay_out { FLAG_PAY_OUT } else { 0 })
                | (if self.pay_in { FLAG_PAY_IN } else { 0 })
                | (if self.reputation { FLAG_REPUTATION } else { 0 }),
        );
        sequence_bits | flag_bits
    }
}

/// EscrowData structure matching the Solidity/TypeScript definition
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowData {
    /// Account funding the escrow
    pub offerer: Address,
    /// Account entitled to claim the funds from the escrow
    pub claimer: Address,

    /// Amount of tokens in the escrow
    pub amount: U256,
    /// Token of the escrow
    pub token: Address,

    /// Misc escrow data flags (payIn, payOut, reputation)
    pub flags: Flags,

    /// Address of the IClaimHandler
    pub claim_handler: Address,
    /// Data provided to the claim handler
    pub claim_data: [u8; 32],

    /// Address of the IRefundHandler
    pub refund_handler: Address,
    /// Data provided to the refund handler
    pub refund_data: [u8; 32],

    /// Security deposit
    pub security_deposit: U256,
    /// Claimer bounty
    pub claimer_bounty: U256,
    /// Deposit token
    pub deposit_token: Address,

    /// ExecutionAction hash commitment
    pub success_action_commitment: [u8; 32],
}

impl EscrowData {
    /// Create a new EscrowData instance
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        offerer: Address,
        claimer: Address,
        amount: U256,
        token: Address,
        pay_out: bool,
        pay_in: bool,
        reputation: bool,
        sequence: u64,
        claim_handler: Address,
        claim_data: [u8; 32],
        refund_handler: Address,
        refund_data: [u8; 32],
        security_deposit: U256,
        claimer_bounty: U256,
        deposit_token: Address,
        success_action_commitment: [u8; 32],
    ) -> Self {
        EscrowData {
            offerer,
            claimer,
            amount,
            token,
            flags: Flags {
                pay_out,
                pay_in,
                reputation,
                sequence,
            },
            claim_handler,
            claim_data,
            refund_handler,
            refund_data,
            security_deposit,
            claimer_bounty,
            deposit_token,
            success_action_commitment,
        }
    }

    /// Deserialize from ABI-encoded bytes (as from raw EVM transaction)
    ///
    /// This matches the Solidity struct encoding:
    /// ```solidity
    /// struct EscrowData {
    ///     address offerer;           // 0x0-0x14  (20 bytes, padded to 32)
    ///     address claimer;           // 0x20-0x34 (20 bytes, padded to 32)
    ///     uint256 amount;            // 0x40-0x5F (32 bytes)
    ///     address token;             // 0x60-0x74 (20 bytes, padded to 32)
    ///     uint256 flags;             // 0x80-0x9F (32 bytes)
    ///     address claimHandler;      // 0xA0-0xB4 (20 bytes, padded to 32)
    ///     bytes32 claimData;         // 0xC0-0xDF (32 bytes)
    ///     address refundHandler;     // 0xE0-0xF4 (20 bytes, padded to 32)
    ///     bytes32 refundData;        // 0x100-0x11F (32 bytes)
    ///     uint256 securityDeposit;   // 0x120-0x13F (32 bytes)
    ///     uint256 claimerBounty;     // 0x140-0x15F (32 bytes)
    ///     address depositToken;      // 0x160-0x174 (20 bytes, padded to 32)
    ///     bytes32 successActionCommitment; // 0x180-0x19F (32 bytes)
    /// }
    /// ```
    pub fn from_abi_encoded(data: &[u8]) -> Result<Self, String> {
        // Total expected size: 13 fields * 32 bytes = 416 bytes
        if data.len() < 416 {
            return Err(format!(
                "Insufficient data for EscrowData: expected at least 416 bytes, got {}",
                data.len()
            ));
        }

        let mut offset = 0;

        // offerer (address, padded to 32 bytes)
        let offerer = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // claimer (address, padded to 32 bytes)
        let claimer = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // amount (uint256)
        let amount = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // token (address, padded to 32 bytes)
        let token = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // flags (uint256)
        let flags_raw = Self::decode_u256(&data[offset..offset + 32])?;
        let flags = Flags::from_u256(flags_raw);
        offset += 32;

        // claimHandler (address, padded to 32 bytes)
        let claim_handler = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // claimData (bytes32)
        let claim_data = Self::decode_bytes32(&data[offset..offset + 32])?;
        offset += 32;

        // refundHandler (address, padded to 32 bytes)
        let refund_handler = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // refundData (bytes32)
        let refund_data = Self::decode_bytes32(&data[offset..offset + 32])?;
        offset += 32;

        // securityDeposit (uint256)
        let security_deposit = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // claimerBounty (uint256)
        let claimer_bounty = Self::decode_u256(&data[offset..offset + 32])?;
        offset += 32;

        // depositToken (address, padded to 32 bytes)
        let deposit_token = Self::decode_address(&data[offset..offset + 32])?;
        offset += 32;

        // successActionCommitment (bytes32)
        let success_action_commitment = Self::decode_bytes32(&data[offset..offset + 32])?;

        Ok(EscrowData {
            offerer,
            claimer,
            amount,
            token,
            flags,
            claim_handler,
            claim_data,
            refund_handler,
            refund_data,
            security_deposit,
            claimer_bounty,
            deposit_token,
            success_action_commitment,
        })
    }

    /// Deserialize from transaction calldata
    /// Assumes the data starts after the function selector (first 4 bytes)
    pub fn from_transaction_calldata(calldata: &[u8]) -> Result<Self, String> {
        // Skip function selector if present (4 bytes)
        let data = if calldata.len() > 4 && calldata.len() % 32 == 4 {
            &calldata[4..]
        } else {
            calldata
        };

        Self::from_abi_encoded(data)
    }

    /// Serialize to ABI-encoded bytes
    pub fn to_abi_encoded(&self) -> Vec<u8> {
        let mut result = Vec::with_capacity(416);

        // offerer
        result.extend_from_slice(&Self::encode_address(self.offerer));

        // claimer
        result.extend_from_slice(&Self::encode_address(self.claimer));

        // amount
        result.extend_from_slice(&Self::encode_u256(self.amount));

        // token
        result.extend_from_slice(&Self::encode_address(self.token));

        // flags
        result.extend_from_slice(&Self::encode_u256(self.flags.to_u256()));

        // claimHandler
        result.extend_from_slice(&Self::encode_address(self.claim_handler));

        // claimData
        result.extend_from_slice(&self.claim_data);

        // refundHandler
        result.extend_from_slice(&Self::encode_address(self.refund_handler));

        // refundData
        result.extend_from_slice(&self.refund_data);

        // securityDeposit
        result.extend_from_slice(&Self::encode_u256(self.security_deposit));

        // claimerBounty
        result.extend_from_slice(&Self::encode_u256(self.claimer_bounty));

        // depositToken
        result.extend_from_slice(&Self::encode_address(self.deposit_token));

        // successActionCommitment
        result.extend_from_slice(&self.success_action_commitment);

        result
    }

    /// Calculate the escrow hash (keccak256 of ABI-encoded data)
    pub fn escrow_hash(&self) -> H256 {
        use web3::signing::keccak256;
        H256::from(keccak256(&self.to_abi_encoded()))
    }

    /// Get the claim data as H256
    pub fn claim_data_hash(&self) -> H256 {
        H256::from(self.claim_data)
    }

    /// Get the refund data as H256
    pub fn refund_data_hash(&self) -> H256 {
        H256::from(self.refund_data)
    }

    /// Check if success action is set (non-zero)
    pub fn has_success_action(&self) -> bool {
        self.success_action_commitment != [0u8; 32]
    }

    /// Helper: Decode address from 32-byte padded value
    fn decode_address(data: &[u8]) -> Result<Address, String> {
        if data.len() != 32 {
            return Err("Address data must be 32 bytes".to_string());
        }
        // Address is in the last 20 bytes (right-padded)
        let mut addr_bytes = [0u8; 20];
        addr_bytes.copy_from_slice(&data[12..32]);
        Ok(Address::from(addr_bytes))
    }

    /// Helper: Decode U256 from 32-byte value
    fn decode_u256(data: &[u8]) -> Result<U256, String> {
        if data.len() != 32 {
            return Err("U256 data must be 32 bytes".to_string());
        }
        Ok(U256::from_big_endian(data))
    }

    /// Helper: Decode bytes32
    fn decode_bytes32(data: &[u8]) -> Result<[u8; 32], String> {
        if data.len() != 32 {
            return Err("bytes32 data must be 32 bytes".to_string());
        }
        let mut result = [0u8; 32];
        result.copy_from_slice(data);
        Ok(result)
    }

    /// Helper: Encode address to 32-byte padded value
    fn encode_address(addr: Address) -> [u8; 32] {
        let mut result = [0u8; 32];
        // Address goes in the last 20 bytes (right-aligned)
        result[12..32].copy_from_slice(addr.as_bytes());
        result
    }

    /// Helper: Encode U256 to 32-byte big-endian value
    fn encode_u256(value: U256) -> [u8; 32] {
        let mut result = [0u8; 32];
        value.to_big_endian(&mut result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn test_flags_encoding_decoding() {
        let flags = Flags {
            pay_out: true,
            pay_in: false,
            reputation: true,
            sequence: 0x1234,
        };

        let encoded = flags.to_u256();
        let decoded = Flags::from_u256(encoded);

        assert_eq!(flags, decoded);
    }

    #[test]
    fn test_escrow_data_creation() {
        let escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [9u8; 32],
        );

        assert_eq!(escrow.amount, U256::from(1_000_000u64));
        assert!(escrow.flags.pay_out);
        assert!(!escrow.flags.pay_in);
        assert!(escrow.flags.reputation);
        assert_eq!(escrow.flags.sequence, 42);
    }

    #[test]
    fn test_escrow_data_roundtrip() {
        let original = EscrowData::new(
            "0x1234567890123456789012345678901234567890"
                .parse()
                .unwrap(),
            "0x0987654321098765432109876543210987654321"
                .parse()
                .unwrap(),
            U256::from(5_000_000u64),
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                .parse()
                .unwrap(),
            true,
            true,
            false,
            999,
            "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd"
                .parse()
                .unwrap(),
            [1u8; 32],
            "0xaabbccddaabbccddaabbccddaabbccddaabbccdd"
                .parse()
                .unwrap(),
            [2u8; 32],
            U256::from(100_000u64),
            U256::from(25_000u64),
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                .parse()
                .unwrap(),
            [3u8; 32],
        );

        let encoded = original.to_abi_encoded();
        assert_eq!(encoded.len(), 416);

        let decoded = EscrowData::from_abi_encoded(&encoded).expect("Failed to decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn test_escrow_hash_consistency() {
        let escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [9u8; 32],
        );

        let hash1 = escrow.escrow_hash();
        let hash2 = escrow.escrow_hash();

        assert_eq!(hash1, hash2, "Hashes should be deterministic");
    }

    #[test]
    fn test_success_action_commitment() {
        let mut escrow = EscrowData::new(
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap(),
            "0x2222222222222222222222222222222222222222"
                .parse()
                .unwrap(),
            U256::from(1_000_000u64),
            "0x3333333333333333333333333333333333333333"
                .parse()
                .unwrap(),
            true,
            false,
            true,
            42,
            "0x4444444444444444444444444444444444444444"
                .parse()
                .unwrap(),
            [5u8; 32],
            "0x6666666666666666666666666666666666666666"
                .parse()
                .unwrap(),
            [7u8; 32],
            U256::from(50_000u64),
            U256::from(10_000u64),
            "0x8888888888888888888888888888888888888888"
                .parse()
                .unwrap(),
            [0u8; 32], // Zero = no success action
        );

        assert!(!escrow.has_success_action());

        escrow.success_action_commitment = [1u8; 32];
        assert!(escrow.has_success_action());
    }

    #[test]
    fn test_from_abi_encoded_minimal() {
        // Create minimal valid data (416 bytes)
        let mut data = vec![0u8; 416];

        // Set offerer at offset 12-32
        let offerer_bytes = hex::decode("1111111111111111111111111111111111111111").unwrap();
        data[12..32].copy_from_slice(&offerer_bytes);

        // Set claimer at offset 44-64 (offset 32 + 12)
        let claimer_bytes = hex::decode("2222222222222222222222222222222222222222").unwrap();
        data[44..64].copy_from_slice(&claimer_bytes);

        let result = EscrowData::from_abi_encoded(&data);
        assert!(result.is_ok());

        let escrow = result.unwrap();
        assert_eq!(
            escrow.offerer,
            "0x1111111111111111111111111111111111111111"
                .parse()
                .unwrap()
        );
    }

    #[test]
    fn test_from_transaction_calldata() {
        // bolt11
        // lntb21u1p57w7ujpp5w9vzr5f5rg426l6pmtumg5kmahw53u985pd4a38k9dn6ycy3hz3sdq2f38xy6t5wvcqzzsxqrrsssp5c5gmwupuu4vhh7gmvlxh32gw6n7a6wf9s9hzx59nrw96sjnf3vas9qxpqysgqyxx5qhuwywdul8z8dkum3qgy6l5rqfanqcpvzwcwek2va2x3c5ej83v2k526g5p4upqztr4gnkvzkaheecvj3u42lfm26ylg60qr5dcqatw2xz

        let mut data = vec![0u8; 676];
        let tx_bytes = hex::decode(
            "07dd7a29000000000000000000000000ba7633f36a86a4f572a918350574c1a\
        44b924ebf000000000000000000000000110caeb55493b119f208b73245464ef5d9a1c39e000000000000000000\
        000000000000000000000000000000000013ac20776c00000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000009222e8d83b4d9225000000000000000600000000\
        00000000000000001120e1eb3049148aebee497331774bfe1f6c174d715821d1341a2aad7f41daf9b452dbeddd4\
        8f0a7a05b5ec4f62b67a26091b8a30000000000000000000000004699450973c21d6fe09e36a8a475eae4d78a31\
        370000000000000000000000000000000000000000000000000000000069ee14200000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        00000000000000000002000000000000000000000000000000000000000000000000000000000069e77cdc00000\
        0000000000000000000000000000000000000000000000000000000028000000000000000000000000000000000\
        000000000000000000000000000000416546c65d2624fba2d98ce98fa149f43856a0c6f892b24e03774c1da0ed8\
        90c427798d9351b293e7d3e4b6cac34779bcba1d3c1a58f0a3dc042247018a53c28a31c00000000000000000000\
        0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        000000000000000",
        )
        .unwrap();
        data.copy_from_slice(&tx_bytes);

        let result = EscrowData::from_transaction_calldata(&data);
        assert!(result.is_ok());
        let escrow = result.unwrap();

        assert_eq!(
            escrow.offerer,
            "0xba7633f36a86a4f572a918350574c1a44b924ebf"
                .parse()
                .unwrap(),
            "Offerer mismatch"
        );
        assert_eq!(
            escrow.claimer,
            "0x110caeb55493b119f208b73245464ef5d9a1c39e"
                .parse()
                .unwrap(),
            "Claimer mismatch"
        );
        assert_eq!(
            escrow.amount,
            U256::from(21630000000000_u64),
            "Amount must be equal"
        );
        assert_eq!(
            escrow.token,
            Address::zero(),
            "Destination token is native token"
        );
        assert!(
            escrow.flags.pay_out || escrow.flags.pay_in,
            "At least one of pay_out or pay_in must be true"
        );
        assert!(
            escrow.claim_handler != Address::zero(),
            "Claim handler must not be zero address"
        );
        assert!(
            escrow.refund_handler != Address::zero(),
            "Refund handler must not be zero address"
        );
        assert!(
            !escrow.has_success_action(),
            "Success action should not be set (all zeros)"
        );

        let payment_hash = H256::from(escrow.claim_data);
        let payment_hash_arr: [u8; 32] = payment_hash.into();

        let hex_formatted = format!("{payment_hash:#x}");
        assert_eq!(
            hex_formatted,
            "0x7158_21d1_341a_2aad_7f41_daf9_b452_dbed_dd48_f0a7_a05b_5ec4_f62b_67a2_6091_b8a3"
                .replace('_', "")
                .to_lowercase()
        );

        let preimage = "373cbb0a28b180d9f9171480f7f73df4d554620a73cccb05d6c6ce0af6a8d8a4";
        let preimage_bytes = hex::decode(preimage).unwrap();

        let mut sha256 = Sha256::new();
        sha256.update(&preimage_bytes);
        let hash: [u8; 32] = sha256.finalize().into();

        assert_eq!(
            hash, payment_hash_arr,
            "Hash must be equal to given preimage"
        );
    }
}
