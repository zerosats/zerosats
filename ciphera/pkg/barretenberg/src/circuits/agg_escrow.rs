//! Backend wrapping for the Noir `agg_escrow` 1-level aggregator.
//!
//! Mirrors [`super::agg_utxo`] but binds the `verification_key` /
//! `verification_key_hash` input slots to the *escrow* leaf VK rather
//! than the utxo one, so the underlying Noir program (which hardcodes
//! `escrow_VK_HASH`) accepts the bundled leaves. The output proof
//! shape is intentionally identical to `agg_utxo`'s -- both produce
//! 18 public-input fields and a 508-field proof body -- so we reuse
//! [`AggUtxoProof`] as the return type, and `agg_agg` tracks the leaf
//! source via [`zk_primitives::AggLeafSource`].
use super::agg_utxo::AggUtxoProofInput;
use super::{ESCROW_VERIFICATION_KEY, ESCROW_VERIFICATION_KEY_HASH};
use crate::Result;
use crate::backend::DefaultBackend;
use crate::circuits::get_bytecode_from_program;
use crate::prove::prove;
use crate::traits::{Prove, Verify};
use crate::util::write_to_temp_file;
use crate::verify::{VerificationKey, VerificationKeyHash, verify};
use element::Base;
use lazy_static::lazy_static;
use noirc_abi::{InputMap, input_parser::InputValue};
use noirc_artifacts::program::ProgramArtifact;
use noirc_driver::CompiledProgram;
use std::path::PathBuf;
use zk_primitives::{
    AggEscrow, AggEscrowProof, AggUtxoProof, AggUtxoProofBytes, AggUtxoPublicInput,
    EscrowProofBundleWithMerkleProofs, MerklePath, bytes_to_elements,
};

const PROGRAM: &str = include_str!("../../../../fixtures/programs/agg_escrow.json");
const KEY: &[u8] = include_bytes!("../../../../fixtures/keys/agg_escrow_key");

lazy_static! {
    static ref PROGRAM_ARTIFACT: ProgramArtifact = serde_json::from_str(PROGRAM).unwrap();
    static ref PROGRAM_COMPILED: CompiledProgram = CompiledProgram::from(PROGRAM_ARTIFACT.clone());
    static ref PROGRAM_PATH: PathBuf = write_to_temp_file(PROGRAM.as_bytes(), ".json");
    static ref BYTECODE: Vec<u8> = get_bytecode_from_program(PROGRAM);
    /// Verification key for the `agg_escrow` 1-level aggregator.
    pub static ref AGG_ESCROW_VERIFICATION_KEY: VerificationKey =
        VerificationKey::from_bytes(KEY).expect("Fail to read verification key");
    /// Poseidon hash of [`AGG_ESCROW_VERIFICATION_KEY`]. Hardcoded into
    /// `agg_agg`'s Noir source alongside `AGG_UTXO_VERIFICATION_KEY_HASH`.
    pub static ref AGG_ESCROW_VERIFICATION_KEY_HASH: VerificationKeyHash = VerificationKeyHash(
        bn254_blackbox_solver::poseidon_hash(&AGG_ESCROW_VERIFICATION_KEY.0).unwrap()
    );
}

const AGG_ESCROW_PUBLIC_INPUTS_COUNT: usize = 18;

impl Prove for AggEscrow {
    type Proof = AggEscrowProof;
    type Result<Proof> = Result<Proof>;

    fn prove(&self) -> Self::Result<Self::Proof> {
        let inputs = InputMap::from(AggEscrowInput::from(self));

        let proof_bytes =
            prove::<DefaultBackend>(&PROGRAM_COMPILED, PROGRAM.as_bytes(), KEY, &inputs, false)?;

        let public_inputs = proof_bytes[..AGG_ESCROW_PUBLIC_INPUTS_COUNT * 32].to_vec();
        let public_inputs = bytes_to_elements(&public_inputs);
        let raw_proof = proof_bytes[AGG_ESCROW_PUBLIC_INPUTS_COUNT * 32..].to_vec();

        assert_eq!(
            public_inputs.len(),
            AGG_ESCROW_PUBLIC_INPUTS_COUNT,
            "Public inputs must be {AGG_ESCROW_PUBLIC_INPUTS_COUNT} elements"
        );
        assert_eq!(
            raw_proof.len(),
            508 * 32,
            "Proof must be 508 elements of 32 bytes"
        );

        Ok(AggEscrowProof(AggUtxoProof {
            proof: AggUtxoProofBytes(raw_proof),
            public_inputs: AggUtxoPublicInput {
                messages: [
                    public_inputs[0],
                    public_inputs[1],
                    public_inputs[2],
                    public_inputs[3],
                    public_inputs[4],
                    public_inputs[5],
                    public_inputs[6],
                    public_inputs[7],
                    public_inputs[8],
                    public_inputs[9],
                    public_inputs[10],
                    public_inputs[11],
                    public_inputs[12],
                    public_inputs[13],
                    public_inputs[14],
                ],
                old_root: public_inputs[15],
                new_root: public_inputs[16],
                commit_hash: public_inputs[17],
            },
        }))
    }
}

impl Verify for AggEscrowProof {
    fn verify(&self) -> Result<()> {
        let inner = self.as_agg_utxo_proof();
        verify::<DefaultBackend>(KEY, &inner.public_inputs.to_bytes(), &inner.proof.0, false)
    }
}

#[derive(Debug, Clone)]
struct AggEscrowInput {
    proofs: [AggUtxoProofInput; 3],
    messages: [Base; 15],
    old_root: Base,
    new_root: Base,
    commit_hash: Base,
}

impl From<&AggEscrow> for AggEscrowInput {
    fn from(agg_escrow: &AggEscrow) -> Self {
        let proofs: Vec<AggUtxoProofInput> = agg_escrow
            .proofs
            .iter()
            .map(escrow_bundle_to_agg_input)
            .collect();
        let messages: [Base; 15] = agg_escrow.messages().map(|e| e.to_base());
        AggEscrowInput {
            proofs: proofs.try_into().unwrap(),
            messages,
            old_root: agg_escrow.old_root.to_base(),
            new_root: agg_escrow.new_root.to_base(),
            commit_hash: agg_escrow.commit_hash().to_base(),
        }
    }
}

fn escrow_bundle_to_agg_input(value: &EscrowProofBundleWithMerkleProofs) -> AggUtxoProofInput {
    let convert_merkle_paths = |merkle_paths: &[MerklePath<161>; 2]| -> [[Base; 160]; 2] {
        let mut paths = [[Base::default(); 160]; 2];
        for (i, mp) in merkle_paths.iter().enumerate() {
            for (j, s) in mp.siblings.iter().enumerate() {
                paths[i][j] = s.to_base();
            }
        }
        paths
    };

    AggUtxoProofInput {
        proof: value
            .escrow_proof
            .proof
            .to_fields()
            .iter()
            .map(|e| e.to_base())
            .collect::<Vec<_>>()
            .try_into()
            .unwrap(),
        input_merkle_paths: convert_merkle_paths(&value.input_merkle_paths),
        output_merkle_paths: convert_merkle_paths(&value.output_merkle_paths),
        input_commitments: value
            .escrow_proof
            .public_inputs
            .input_commitments
            .map(|e| e.to_base()),
        output_commitments: value
            .escrow_proof
            .public_inputs
            .output_commitments
            .map(|e| e.to_base()),
        utxo_kind: value.escrow_proof.kind().to_element().to_base(),
    }
}

impl From<AggEscrowInput> for InputMap {
    fn from(value: AggEscrowInput) -> Self {
        let mut map = InputMap::new();

        map.insert(
            "verification_key".to_owned(),
            InputValue::Vec(
                ESCROW_VERIFICATION_KEY
                    .0
                    .iter()
                    .cloned()
                    .map(InputValue::Field)
                    .collect(),
            ),
        );
        map.insert(
            "verification_key_hash".to_owned(),
            InputValue::Field(ESCROW_VERIFICATION_KEY_HASH.0),
        );

        map.insert(
            "proofs".to_owned(),
            InputValue::Vec(value.proofs.map(InputValue::from).to_vec()),
        );
        map.insert(
            "messages".to_owned(),
            InputValue::Vec(value.messages.map(InputValue::Field).to_vec()),
        );
        map.insert("old_root".to_owned(), InputValue::Field(value.old_root));
        map.insert("new_root".to_owned(), InputValue::Field(value.new_root));
        map.insert(
            "commit_hash".to_owned(),
            InputValue::Field(value.commit_hash),
        );

        map
    }
}
