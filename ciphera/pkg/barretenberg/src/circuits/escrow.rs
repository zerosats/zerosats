//! Backend wrapping for the Noir `escrow` leaf circuit.
//!
//! Mirrors `circuits::utxo` but pulls `EscrowInputNote`s into the input
//! map -- each input note carries a `spend_type`, a private key, an
//! optional 32-byte preimage and a `TimeProof` witness. The public
//! inputs and proof size are intentionally the same shape as the
//! `utxo` circuit so the resulting proof can be aggregated by
//! `agg_escrow` and then dropped into the same `agg_agg` slot a
//! `utxo` / `agg_utxo` proof would occupy.
use super::note::BNote;
use crate::{
    Result,
    backend::DefaultBackend,
    circuits::get_bytecode_from_program,
    prove::prove,
    traits::{Prove, Verify},
    util::write_to_temp_file,
    verify::{VerificationKey, VerificationKeyHash, verify},
};
use element::Base;
use lazy_static::lazy_static;
use noirc_abi::{InputMap, input_parser::InputValue};
use noirc_artifacts::program::ProgramArtifact;
use noirc_driver::CompiledProgram;
use std::collections::BTreeMap;
use std::path::PathBuf;
use zk_primitives::{
    ESCROW_PROOF_SIZE, ESCROW_PUBLIC_INPUTS_COUNT, Escrow, EscrowInputNote, EscrowProof,
    EscrowProofBytes, TimeLock, TimeProof, UtxoPublicInput, bytes_to_elements,
};

const PROGRAM: &str = include_str!("../../../../fixtures/programs/escrow.json");
const KEY: &[u8] = include_bytes!("../../../../fixtures/keys/escrow_key");

lazy_static! {
    static ref PROGRAM_ARTIFACT: ProgramArtifact = serde_json::from_str(PROGRAM).unwrap();
    static ref PROGRAM_COMPILED: CompiledProgram = CompiledProgram::from(PROGRAM_ARTIFACT.clone());
    static ref PROGRAM_PATH: PathBuf = write_to_temp_file(PROGRAM.as_bytes(), ".json");
    static ref BYTECODE: Vec<u8> = get_bytecode_from_program(PROGRAM);
    /// Verification key for the escrow leaf circuit.
    pub static ref ESCROW_VERIFICATION_KEY: VerificationKey =
        VerificationKey::from_bytes(KEY).expect("Fail to read verification key");
    /// Poseidon hash of the escrow VK. Hardcoded into `agg_escrow`'s
    /// Noir source to gate leaf admissibility.
    pub static ref ESCROW_VERIFICATION_KEY_HASH: VerificationKeyHash = VerificationKeyHash(
        bn254_blackbox_solver::poseidon_hash(&ESCROW_VERIFICATION_KEY.0).unwrap()
    );
}

impl Prove for Escrow {
    type Proof = EscrowProof;
    type Result<Proof> = Result<Proof>;

    fn prove(&self) -> Self::Result<Self::Proof> {
        let inputs = InputMap::from(EscrowProveInput::from(self));

        let proof_bytes =
            prove::<DefaultBackend>(&PROGRAM_COMPILED, PROGRAM.as_bytes(), KEY, &inputs, false)?;

        let public_inputs = proof_bytes[..ESCROW_PUBLIC_INPUTS_COUNT * 32].to_vec();
        let public_inputs = bytes_to_elements(&public_inputs);
        let raw_proof = proof_bytes[ESCROW_PUBLIC_INPUTS_COUNT * 32..].to_vec();

        assert_eq!(
            raw_proof.len(),
            ESCROW_PROOF_SIZE * 32,
            "Proof must be {ESCROW_PROOF_SIZE} elements of 32 bytes"
        );

        Ok(EscrowProof {
            proof: EscrowProofBytes(raw_proof),
            public_inputs: UtxoPublicInput {
                input_commitments: [public_inputs[0], public_inputs[1]],
                output_commitments: [public_inputs[2], public_inputs[3]],
                messages: [
                    public_inputs[4],
                    public_inputs[5],
                    public_inputs[6],
                    public_inputs[7],
                    public_inputs[8],
                ],
            },
        })
    }
}

impl Verify for EscrowProof {
    fn verify(&self) -> Result<()> {
        verify::<DefaultBackend>(KEY, &self.public_inputs.to_bytes(), &self.proof.0, false)
    }
}

/// Witness layout for a single input note as the Noir `escrow` circuit
/// expects it -- mirrors `common::EscrowNote` in `noir/common/src/lib.nr`.
#[derive(Debug, Clone)]
pub struct BEscrowInputNote {
    /// The note being spent.
    pub note: BNote,
    /// Selects the spend-condition branch (`0` poseidon-key, `1`
    /// signature32, `2` signature32sha, `3` HTLC).
    pub spend_type: Base,
    /// Poseidon-key spend secret. Zero for the non-key paths.
    pub secret_key: Base,
    /// 32-byte preimage witness for the preimage-based paths (kind 5,
    /// kind 6, and the kind-8 hash branch). Zero on key/refund paths.
    pub preimage: [u8; 32],
    /// PoW chain witness for the timelocked paths.
    pub time_proof: BTimeProof,
}

impl From<&EscrowInputNote> for BEscrowInputNote {
    fn from(value: &EscrowInputNote) -> Self {
        BEscrowInputNote {
            note: BNote::from(&value.note),
            spend_type: Base::from(u128::from(value.spend_type)),
            secret_key: value.secret_key.to_base(),
            preimage: value.preimage,
            time_proof: BTimeProof::from(&value.time_proof),
        }
    }
}

impl From<BEscrowInputNote> for InputValue {
    fn from(value: BEscrowInputNote) -> Self {
        InputValue::Struct(BTreeMap::from([
            ("note".to_owned(), value.note.into()),
            ("spend_type".to_owned(), InputValue::Field(value.spend_type)),
            ("secret_key".to_owned(), InputValue::Field(value.secret_key)),
            ("preimage".to_owned(), bytes_to_input_value(&value.preimage)),
            ("time_proof".to_owned(), value.time_proof.into()),
        ]))
    }
}

/// Noir-side mirror of [`zk_primitives::TimeLock`].
#[derive(Debug, Clone)]
pub struct BTimeLock {
    /// Bitcoin block hash anchor (big-endian, as on chain).
    pub zero_block: [u8; 32],
    /// Number of additional PoW blocks required.
    pub n_blocks: Base,
}

impl From<&TimeLock> for BTimeLock {
    fn from(lock: &TimeLock) -> Self {
        BTimeLock {
            zero_block: lock.zero_block,
            n_blocks: lock.n_blocks.to_base(),
        }
    }
}

impl From<BTimeLock> for InputValue {
    fn from(lock: BTimeLock) -> Self {
        InputValue::Struct(BTreeMap::from([
            ("zero_block".to_owned(), bytes_to_input_value(&lock.zero_block)),
            ("n_blocks".to_owned(), InputValue::Field(lock.n_blocks)),
        ]))
    }
}

/// Noir-side mirror of [`zk_primitives::TimeProof`]. Carries the lock
/// it extends plus the headers chaining from `lock.zero_block`.
#[derive(Debug, Clone)]
pub struct BTimeProof {
    /// The lock these headers extend.
    pub lock: BTimeLock,
    /// Two 80-byte block headers chaining from the anchor.
    pub headers: [[u8; 80]; 2],
}

impl From<&TimeProof> for BTimeProof {
    fn from(value: &TimeProof) -> Self {
        BTimeProof {
            lock: BTimeLock::from(&value.lock),
            headers: value.headers,
        }
    }
}

impl From<BTimeProof> for InputValue {
    fn from(value: BTimeProof) -> Self {
        let headers = InputValue::Vec(
            value
                .headers
                .iter()
                .map(|h| bytes_to_input_value(h))
                .collect(),
        );
        InputValue::Struct(BTreeMap::from([
            ("lock".to_owned(), value.lock.into()),
            ("headers".to_owned(), headers),
        ]))
    }
}

fn bytes_to_input_value(bytes: &[u8]) -> InputValue {
    InputValue::Vec(
        bytes
            .iter()
            .map(|b| InputValue::Field(Base::from(u128::from(*b))))
            .collect(),
    )
}

#[derive(Debug, Clone)]
struct EscrowProveInput {
    input_notes: [BEscrowInputNote; 2],
    output_notes: [BNote; 2],
    pmessage4: Base,
    commitments: [Base; 4],
    messages: [Base; 5],
}

impl From<&Escrow> for EscrowProveInput {
    fn from(escrow: &Escrow) -> Self {
        EscrowProveInput {
            input_notes: escrow
                .input_notes
                .iter()
                .map(BEscrowInputNote::from)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
            output_notes: escrow
                .output_notes
                .iter()
                .map(BNote::from)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
            pmessage4: escrow.messages()[4].to_base(),
            commitments: [
                escrow.input_notes[0].note.commitment().to_base(),
                escrow.input_notes[1].note.commitment().to_base(),
                escrow.output_notes[0].commitment().to_base(),
                escrow.output_notes[1].commitment().to_base(),
            ],
            messages: escrow.messages().map(|e| e.to_base()),
        }
    }
}

impl From<EscrowProveInput> for InputMap {
    fn from(input: EscrowProveInput) -> Self {
        let mut map = InputMap::new();

        map.insert(
            "input_notes".to_owned(),
            InputValue::Vec(input.input_notes.map(InputValue::from).to_vec()),
        );
        map.insert(
            "output_notes".to_owned(),
            InputValue::Vec(input.output_notes.map(InputValue::from).to_vec()),
        );
        // The escrow circuit currently ignores pmessage4 (denoted
        // `_pmessage4` in main.nr), but we still need to bind it so the
        // public/private input shape matches the utxo circuit.
        map.insert("_pmessage4".to_owned(), InputValue::Field(input.pmessage4));
        map.insert(
            "commitments".to_owned(),
            InputValue::Vec(input.commitments.map(InputValue::Field).to_vec()),
        );
        map.insert(
            "messages".to_owned(),
            InputValue::Vec(input.messages.map(InputValue::Field).to_vec()),
        );

        map
    }
}
