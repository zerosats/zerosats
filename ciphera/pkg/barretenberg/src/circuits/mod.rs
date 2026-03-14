mod agg_agg;
mod agg_utxo;
mod migrate;
mod note;
mod points;
mod signature;
#[cfg(test)]
mod tests;
mod utxo;

use std::io::Read;

use acvm::AcirField;
pub use agg_utxo::*;
use base64::Engine;
use flate2::read::GzDecoder;
pub use utxo::*;

/// Parse a binary VK file (N * 32 bytes) into field elements.
/// BB 4.0 outputs VKs as concatenated 32-byte big-endian field elements.
fn vk_binary_to_fields(vk_bytes: &[u8]) -> Vec<element::Base> {
    assert!(
        vk_bytes.len() % 32 == 0,
        "VK binary must be a multiple of 32 bytes, got {}",
        vk_bytes.len()
    );
    vk_bytes
        .chunks_exact(32)
        .map(|chunk| acvm::FieldElement::from_be_bytes_reduce(chunk))
        .collect()
}

fn get_bytecode_from_program(program_json: &str) -> Vec<u8> {
    let program: serde_json::Value = serde_json::from_str(program_json)
        .expect("failed to parse program JSON");
    let bytecode_base64 = program
        .get("bytecode")
        .expect("program JSON missing 'bytecode' field")
        .as_str()
        .expect("program 'bytecode' field is not a string");
    let bytecode_gzipped = base64::engine::general_purpose::STANDARD
        .decode(bytecode_base64)
        .expect("failed to base64-decode bytecode");
    let mut decoder = GzDecoder::new(&bytecode_gzipped[..]);
    let mut bytecode = Vec::new();
    decoder
        .read_to_end(&mut bytecode)
        .expect("failed to gzip-decompress bytecode");
    bytecode
}
