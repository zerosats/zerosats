use barretenberg::verify::VerificationKey;
use bn254_blackbox_solver::poseidon_hash;
use element::Element;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <vk>", args[0]);
        std::process::exit(1);
    }

    let vk_path = &args[1];

    if !Path::new(vk_path).exists() {
        eprintln!("Error: File {vk_path} does not exist");
        std::process::exit(1);
    }

    let vk_bytes = match fs::read(vk_path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error reading file {vk_path}: {e}");
            std::process::exit(1);
        }
    };

    if vk_bytes.len() % 32 != 0 {
        eprintln!(
            "Error: vk file {vk_path} length {} is not a multiple of 32",
            vk_bytes.len()
        );
        std::process::exit(1);
    }

    let verification_key = VerificationKey::from_bytes(&vk_bytes);

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
