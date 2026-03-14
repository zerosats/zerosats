use crate::{Result, backend::{Backend, VerifierTarget}, execute::execute_program_and_decode};
use noirc_abi::InputMap;
use noirc_artifacts::program::CompiledProgram;

pub fn prove<B: Backend>(
    compiled_program: &CompiledProgram,
    program: &[u8],
    bytecode: &[u8],
    key: &[u8],
    inputs_map: &InputMap,
    target: VerifierTarget,
) -> Result<Vec<u8>> {
    let results = execute_program_and_decode(compiled_program, inputs_map, false)?;

    let witness = results.witness_stack.serialize()?;

    B::prove(program, bytecode, key, &witness, target)
}
