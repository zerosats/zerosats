use super::{AGG_UTXO_VERIFICATION_KEY, AGG_UTXO_VERIFICATION_KEY_HASH};
use crate::Result;
use crate::backend::{DefaultBackend, VerifierTarget};
use crate::circuits::get_bytecode_from_program;
use crate::verify::{VerificationKey, VerificationKeyHash, verify};
use crate::{
    prove::prove,
    traits::{Prove, Verify},
};
use element::Base;
use lazy_static::lazy_static;
use noirc_abi::{InputMap, input_parser::InputValue};
use noirc_artifacts::program::ProgramArtifact;
use noirc_artifacts::program::CompiledProgram;
use std::collections::BTreeMap;
use zk_primitives::{
    AggAgg, AggAggProof, AggAggProofBytes, AggAggPublicInput, AggUtxoProof, ToBytes,
    bytes_to_elements,
};

const PROGRAM: &str = include_str!("../../../../fixtures/programs/agg_agg.json");
const KEY: &[u8] = include_bytes!("../../../../fixtures/keys/agg_agg_key");

lazy_static! {
    static ref PROGRAM_ARTIFACT: ProgramArtifact = serde_json::from_str(PROGRAM).unwrap();
    static ref PROGRAM_COMPILED: CompiledProgram = CompiledProgram::from(PROGRAM_ARTIFACT.clone());
    static ref BYTECODE: Vec<u8> = get_bytecode_from_program(PROGRAM);
    pub static ref AGG_AGG_VERIFICATION_KEY: VerificationKey =
        VerificationKey(super::vk_binary_to_fields(KEY));
    pub static ref AGG_AGG_VERIFICATION_KEY_HASH: VerificationKeyHash = VerificationKeyHash(
        hash::poseidon_hash(&AGG_AGG_VERIFICATION_KEY.0).unwrap()
    );
}

// agg_agg public inputs layout:
// old_root (1) + new_root (1) + commit_hash (1) + messages (2 proofs * 15 messages = 30) = 33
const AGG_AGG_PUBLIC_INPUTS_COUNT: usize = 1 + 1 + 1 + 30;
const AGG_AGG_PROOF_SIZE: usize = 508;

impl Prove for AggAgg {
    type Proof = AggAggProof;
    type Result<Proof> = Result<Proof>;

    fn prove(&self) -> Self::Result<Self::Proof> {
        let inputs = InputMap::from(AggAggInput::from(self));

        let proof_bytes = prove::<DefaultBackend>(
            &PROGRAM_COMPILED,
            PROGRAM.as_bytes(),
            &BYTECODE,
            KEY,
            &inputs,
            VerifierTarget::Evm,
        )?;
        let public_inputs_bytes = proof_bytes[..AGG_AGG_PUBLIC_INPUTS_COUNT * 32].to_vec();
        let public_inputs = bytes_to_elements(&public_inputs_bytes);
        let raw_proof = proof_bytes[AGG_AGG_PUBLIC_INPUTS_COUNT * 32..].to_vec();

        assert_eq!(
            public_inputs.len(),
            AGG_AGG_PUBLIC_INPUTS_COUNT,
            "Public inputs must be {AGG_AGG_PUBLIC_INPUTS_COUNT} elements"
        );
        assert_eq!(
            raw_proof.len(),
            AGG_AGG_PROOF_SIZE * 32,
            "Proof must be {AGG_AGG_PROOF_SIZE} elements of 32 bytes, got {} bytes",
            raw_proof.len()
        );

        let p = AggAggProof {
            proof: AggAggProofBytes(raw_proof),
            public_inputs: AggAggPublicInput {
                old_root: public_inputs[0],
                new_root: public_inputs[1],
                commit_hash: public_inputs[2],
                messages: public_inputs[3..3 + 2 * 15].to_vec(),
            },
            kzg: vec![],
        };
        Ok(p)
    }
}

impl Verify for AggAggProof {
    fn verify(&self) -> Result<()> {
        verify::<DefaultBackend>(KEY, &self.to_bytes(), VerifierTarget::Evm, AGG_AGG_PUBLIC_INPUTS_COUNT * 32)
    }
}

#[derive(Debug, Clone)]
pub struct AggAggInput {
    pub proofs: [UtxoAggProof; 2],
    pub messages: [Base; 2 * 15],
    pub old_root: Base,
    pub new_root: Base,
    pub commit_hash: Base,
}

impl From<&AggAgg> for AggAggInput {
    fn from(agg_agg: &AggAgg) -> Self {
        AggAggInput {
            proofs: [
                UtxoAggProof::from(&agg_agg.proofs[0]),
                UtxoAggProof::from(&agg_agg.proofs[1]),
            ],
            messages: extract_messages(&agg_agg.proofs),
            old_root: agg_agg.old_root().to_base(),
            new_root: agg_agg.new_root().to_base(),
            commit_hash: agg_agg.commit_hash().to_base(),
        }
    }
}

fn extract_messages(agg_agg_proofs: &[AggUtxoProof; 2]) -> [Base; 2 * 15] {
    agg_agg_proofs
        .iter()
        .flat_map(|proof| {
            proof
                .public_inputs
                .messages
                .iter()
                .map(|message| message.to_base())
        })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap()
}

impl From<AggAggInput> for InputMap {
    fn from(value: AggAggInput) -> Self {
        let mut map = InputMap::new();

        // Should be static
        map.insert(
            "verification_key".to_owned(),
            InputValue::Vec(
                AGG_UTXO_VERIFICATION_KEY
                    .0
                    .iter()
                    .cloned()
                    .map(InputValue::Field)
                    .collect(),
            ),
        );
        map.insert(
            "verification_key_hash".to_owned(),
            InputValue::Field(AGG_UTXO_VERIFICATION_KEY_HASH.0),
        );

        map.insert(
            "utxo_agg_proofs".to_owned(),
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

#[derive(Debug, Clone)]
pub struct UtxoAggProof {
    pub proof: [Base; 508],
    pub old_root: Base,
    pub new_root: Base,
    pub commit_hash: Base,
}

impl From<&AggUtxoProof> for UtxoAggProof {
    fn from(value: &AggUtxoProof) -> Self {
        UtxoAggProof {
            proof: value
                .proof
                .to_fields()
                .iter()
                .map(|e| e.to_base())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
            old_root: value.public_inputs.old_root.to_base(),
            new_root: value.public_inputs.new_root.to_base(),
            commit_hash: value.public_inputs.commit_hash.to_base(),
        }
    }
}

impl From<UtxoAggProof> for InputValue {
    fn from(value: UtxoAggProof) -> Self {
        let mut struct_ = BTreeMap::new();

        struct_.insert(
            "proof".to_owned(),
            InputValue::Vec(value.proof.map(InputValue::Field).to_vec()),
        );
        struct_.insert("old_root".to_owned(), InputValue::Field(value.old_root));
        struct_.insert("new_root".to_owned(), InputValue::Field(value.new_root));
        struct_.insert(
            "commit_hash".to_owned(),
            InputValue::Field(value.commit_hash),
        );

        InputValue::Struct(struct_)
    }
}
