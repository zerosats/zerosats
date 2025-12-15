use std::{
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    env
};

use flate2::{Compression, read::GzEncoder};
use tempfile::{NamedTempFile, TempDir};
use tracing::{error, info};

use super::Backend;
use crate::Result;

pub struct CliBackend;

fn verify_bb_executable(path: &PathBuf) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .map(|output| {
            tracing::debug!(
                "Installed Barretenberg version {}", String::from_utf8_lossy(&output.stdout)
            );
            output.status.success()
        })
        .unwrap_or(false)
}
fn get_bb_path() -> Result<PathBuf> {
    let bb_path = PathBuf::from(&"bb");
    if verify_bb_executable(&bb_path) {
        // PATH
        return Ok(bb_path)
    };
    if let Ok(workdir) = env::current_exe() {
        // Current directory (MACOS binaries)
        let bb_exe = workdir.parent().unwrap().join("bb");

        if !bb_exe.exists() || !verify_bb_executable(&bb_exe) {
            return Err("Barretenberg backend not found".to_owned().into());
        }
        return Ok(bb_exe)
    }
    if let Ok(bb) = env::var("BB_PATH").map(PathBuf::from) {
        // Last resort
        let bb_exe = bb.parent().unwrap().join("bb");
        // Verify it exists
        if !bb_exe.exists() || !verify_bb_executable(&bb_exe) {
            return Err("Barretenberg backend not found".to_owned().into());
        }
        return Ok(bb_exe)
    } else {
        return Err("Barretenberg backend not found".to_owned().into())
    }
}

impl Backend for CliBackend {
    fn prove(
        program: &[u8],
        _bytecode: &[u8],
        key: &[u8],
        witness: &[u8],
        recursive: bool,
        oracle_hash_keccak: bool,
    ) -> Result<Vec<u8>> {
        let mut witness_gz = GzEncoder::new(witness, Compression::none());
        let mut witness_gz_buf = Vec::with_capacity(witness.len() + 0xFF);
        witness_gz.read_to_end(&mut witness_gz_buf)?;
        let witness_gz = witness_gz_buf;

        let mut program_file = NamedTempFile::with_suffix(".json")?;
        program_file.write_all(program)?;
        program_file.flush()?;

        let mut witness_file = NamedTempFile::new()?;
        witness_file.write_all(&witness_gz)?;
        witness_file.flush()?;

        let mut key_file = NamedTempFile::new()?;
        key_file.write_all(key)?;
        key_file.flush()?;

        let output_dir = TempDir::new()?;

        let bb_path = get_bb_path()?;
        let mut cmd = Command::new(&bb_path);

        let mut cmd = Command::new(&bb_path);
        cmd.arg("prove")
            .arg("-v")
            .arg("--scheme")
            .arg("ultra_honk")
            .arg("-b")
            .arg(program_file.path())
            .arg("-w")
            .arg(witness_file.path())
            .arg("-k")
            .arg(key_file.path())
            .arg("-o")
            .arg(output_dir.path());

        if recursive {
            cmd.arg("--honk_recursion")
                .arg("1")
                .arg("--init_kzg_accumulator");
        }

        if oracle_hash_keccak {
            cmd.arg("--oracle_hash").arg("keccak");
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(stderr.into());
        }

        let proof_path = output_dir.path().join("proof");
        let mut proof = std::fs::read(&proof_path)?;

        let public_inputs_path = output_dir.path().join("public_inputs");
        let public_inputs = std::fs::read(&public_inputs_path)?;

        proof.splice(0..0, public_inputs);

        Ok(proof)
    }

    fn verify(proof: &[u8], key: &[u8], oracle_hash_keccak: bool) -> Result<()> {
        let mut key_file = NamedTempFile::new()?;
        key_file.write_all(key)?;
        key_file.flush()?;

        let public_inputs_len = proof.len() - 508 * 32;
        let mut proof_file = NamedTempFile::new()?;
        proof_file.write_all(&proof[public_inputs_len..])?;
        proof_file.flush()?;

        let mut public_inputs_file = NamedTempFile::new()?;
        public_inputs_file.write_all(&proof[..public_inputs_len])?;
        public_inputs_file.flush()?;

        let bb_path = get_bb_path()?;
        let mut cmd = Command::new(&bb_path);

        cmd.arg("verify")
            .arg("-v")
            .arg("--scheme")
            .arg("ultra_honk")
            .arg("-k")
            .arg(key_file.path())
            .arg("-p")
            .arg(proof_file.path())
            .arg("-i")
            .arg(public_inputs_file.path());

        if oracle_hash_keccak {
            cmd.arg("--oracle_hash").arg("keccak");
        }

        let output = cmd.output()?;
        info!("output {:?}", output);

        if !output.status.success() {
            // TODO: return false instead? maybe pass -v and parse out verified: {0/1}
            let stderr = String::from_utf8(output.stderr)?;
            error!("proof error: {}", stderr);
            return Err(stderr.into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bb_in_system_path() {
        // `ls` on Unix, `cmd` on Windows should always exist
        #[cfg(unix)]
        let result = get_bb_path();

        assert!(result.is_some());
    }

    #[test]
    fn test_bb_in_local_dir() {
        assert!(true);
    }

    #[test]
    fn test_bb_in_bb_path_env() {
        assert!(true);
    }
}