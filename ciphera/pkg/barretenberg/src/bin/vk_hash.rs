use acvm::AcirField;
use barretenberg::verify::VerificationKey;
use element::Element;
use hash::poseidon_hash;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <vk_binary_file>", args[0]);
        std::process::exit(1);
    }

    let vk_path = &args[1];

    if !Path::new(vk_path).exists() {
        eprintln!("Error: File {vk_path} does not exist");
        std::process::exit(1);
    }

    let vk_data = match fs::read(vk_path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error reading file {vk_path}: {e}");
            std::process::exit(1);
        }
    };

    // BB 4.0 VK format: concatenated 32-byte big-endian field elements
    if vk_data.len() % 32 != 0 {
        eprintln!(
            "Error: VK file size ({}) is not a multiple of 32 bytes",
            vk_data.len()
        );
        std::process::exit(1);
    }

    let vk_fields: Vec<_> = vk_data
        .chunks_exact(32)
        .map(|chunk| acvm::FieldElement::from_be_bytes_reduce(chunk))
        .collect();

    let verification_key = VerificationKey(vk_fields);

    let hash = match poseidon_hash(&verification_key.0) {
        Ok(hash) => hash,
        Err(e) => {
            eprintln!("Error computing Poseidon hash: {e}");
            std::process::exit(1);
        }
    };

    let hash_u256 = Element::from_base(hash).to_u256();
    let hash_hex = format!("0x{hash_u256:064x}");

    println!("u256: {hash_u256}");
    println!("hex: {hash_hex}");
}
