use std::path::PathBuf;

use element::Base;
use lazy_static::lazy_static;
use noirc_abi::InputMap;
use noirc_artifacts::program::ProgramArtifact;
use noirc_driver::CompiledProgram;
use zk_primitives::{
    Signature32Sha, Signature32ShaProof, Signature32ShaProofBytes, Signature32ShaPublicInput,
    bytes_to_elements,
};

use crate::{
    Result,
    backend::DefaultBackend,
    circuits::get_bytecode_from_program,
    prove::prove,
    traits::{Prove, Verify},
    util::write_to_temp_file,
    verify::verify,
};

const PROGRAM: &str = include_str!("../../../../fixtures/programs/signature32sha.json");
const KEY: &[u8] = include_bytes!("../../../../fixtures/keys/signature32sha_key");

lazy_static! {
    static ref PROGRAM_ARTIFACT: ProgramArtifact = serde_json::from_str(PROGRAM).unwrap();
    static ref PROGRAM_COMPILED: CompiledProgram = CompiledProgram::from(PROGRAM_ARTIFACT.clone());
    static ref PROGRAM_PATH: PathBuf = write_to_temp_file(PROGRAM.as_bytes(), ".json");
    static ref BYTECODE: Vec<u8> = get_bytecode_from_program(PROGRAM);
}

// Two public inputs: poseidon address and message.
const SIGNATURE_PUBLIC_INPUTS_COUNT: usize = 2;

impl Prove for Signature32Sha {
    type Proof = Signature32ShaProof;
    type Result<Proof> = Result<Proof>;

    fn prove(&self) -> Self::Result<Self::Proof> {
        let inputs = InputMap::from(Signature32ShaInput::from(self));

        let proof_bytes = prove::<DefaultBackend>(
            &PROGRAM_COMPILED,
            &PROGRAM.as_bytes(),
            KEY,
            &inputs,
            false,
        )?;

        let public_inputs = proof_bytes[..SIGNATURE_PUBLIC_INPUTS_COUNT * 32].to_vec();
        let public_inputs = bytes_to_elements(&public_inputs);
        let raw_proof = proof_bytes[SIGNATURE_PUBLIC_INPUTS_COUNT * 32..].to_vec();

        let proof = Signature32ShaProof {
            proof: Signature32ShaProofBytes(raw_proof),
            public_inputs: Signature32ShaPublicInput {
                address: public_inputs[0],
                message: public_inputs[1],
            },
        };

        Ok(proof)
    }
}

impl Verify for Signature32ShaProof {
    fn verify(&self) -> Result<()> {
        verify::<DefaultBackend>(KEY, &self.public_inputs.to_bytes(), &self.proof.0, false)
    }
}

#[derive(Debug, Clone)]
struct Signature32ShaInput {
    preimage: [u8; 32],
    sha_hash: [u8; 32],
    message: Base,
    message_hash: Base,
    poseidon_hash: Base,
}

impl From<&Signature32Sha> for Signature32ShaInput {
    fn from(signature: &Signature32Sha) -> Self {
        Signature32ShaInput {
            preimage: signature.preimage,
            sha_hash: signature.sha_hash(),
            message: signature.message.to_base(),
            message_hash: signature.message_hash().to_base(),
            poseidon_hash: signature.address().to_base(),
        }
    }
}

fn bytes_to_input_value(bytes: &[u8]) -> noirc_abi::input_parser::InputValue {
    noirc_abi::input_parser::InputValue::Vec(
        bytes
            .iter()
            .map(|&b| noirc_abi::input_parser::InputValue::Field(Base::from(u128::from(b))))
            .collect(),
    )
}

impl From<Signature32ShaInput> for InputMap {
    fn from(input: Signature32ShaInput) -> Self {
        let mut map = InputMap::new();

        map.insert("preimage".to_owned(), bytes_to_input_value(&input.preimage));
        map.insert("sha_hash".to_owned(), bytes_to_input_value(&input.sha_hash));
        map.insert(
            "message_hash".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.message_hash),
        );
        map.insert(
            "poseidon_hash".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.poseidon_hash),
        );
        map.insert(
            "message".to_owned(),
            noirc_abi::input_parser::InputValue::Field(input.message),
        );

        map
    }
}

#[cfg(test)]
mod tests {
    use element::Element;

    use super::*;

    #[test]
    fn test_signature32sha_proof_generation_and_verification() {
        let preimage = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let message = Element::from(101u64);
        let signature = Signature32Sha { preimage, message };

        let proof = signature.prove().unwrap();

        proof.verify().expect("Verification failed");

        assert_eq!(proof.public_inputs.address, signature.address());
        assert_eq!(proof.public_inputs.message, message);
    }

    #[test]
    fn test_signature32sha_input_conversion() {
        let preimage = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let message = Element::from(101u64);
        let signature = Signature32Sha { preimage, message };

        let input = Signature32ShaInput::from(&signature);

        assert_eq!(input.preimage, preimage);
        assert_eq!(input.message, message.to_base());
        assert_eq!(input.poseidon_hash, signature.address().to_base());
        assert_eq!(input.message_hash, signature.message_hash().to_base());
        // Matches the SHA-256 constant baked into the Noir signature32sha test.
        let expected_sha: [u8; 32] = [
            174, 33, 108, 46, 245, 36, 122, 55, 130, 193, 53, 239, 162, 121, 163, 228, 205, 198,
            16, 148, 39, 15, 93, 43, 229, 140, 98, 4, 183, 166, 18, 201,
        ];
        assert_eq!(input.sha_hash, expected_sha);
    }
}
