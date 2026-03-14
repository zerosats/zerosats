use std::{
    env,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
};

use dirs::home_dir;
use flate2::{Compression, read::GzEncoder};
use tempfile::{NamedTempFile, TempDir};
use tracing::{error, info, warn};

use super::{Backend, VerifierTarget};
use crate::Result;

pub struct CliBackend;

fn verify_bb_executable(path: &PathBuf) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .map(|output| {
            tracing::debug!(
                "Installed Barretenberg version {}",
                String::from_utf8_lossy(&output.stdout)
            );
            output.status.success()
        })
        .unwrap_or(false)
}
fn get_bb_path() -> Result<PathBuf> {
    if let Some(path) = home_dir() {
        // bb is in home directory - standard setup
        let bb_exe = path.join(".bb/bb");
        if bb_exe.exists() && verify_bb_executable(&bb_exe) {
            return Ok(bb_exe);
        }
    };
    if let Ok(workdir) = env::current_exe() {
        // Current directory (MACOS binaries)
        let bb_exe = workdir.parent().unwrap().join("bb");
        if bb_exe.exists() && verify_bb_executable(&bb_exe) {
            return Ok(bb_exe);
        }
    };
    if let Ok(bb) = env::var("BB_PATH").map(PathBuf::from) {
        // Last resort
        let bb_exe = bb.join("bb");
        // Verify it exists
        if bb_exe.exists() && verify_bb_executable(&bb_exe) {
            return Ok(bb_exe);
        }
    }
    // eventually searching in PATH
    let which_result = Command::new("which").arg("bb").output()?;

    if which_result.status.success() {
        let bb_path = String::from_utf8_lossy(&which_result.stdout)
            .into_owned()
            .replace('\n', "");
        let bb_exe = PathBuf::from(&bb_path);
        if bb_exe.exists() && verify_bb_executable(&bb_exe) {
            return Ok(PathBuf::from(&bb_exe));
        }
    }

    Err("Barretenberg backend not found".to_owned().into())
}

impl Backend for CliBackend {
    fn prove(
        program: &[u8],
        _bytecode: &[u8],
        _key: &[u8],
        witness: &[u8],
        target: VerifierTarget,
    ) -> Result<Vec<u8>> {
        let mut program_file = NamedTempFile::with_suffix(".json")?;
        program_file.write_all(program)?;
        program_file.flush()?;

        // witness is already gzipped by WitnessStack::serialize()
        let mut witness_file = NamedTempFile::new()?;
        witness_file.write_all(witness)?;
        witness_file.flush()?;

        let output_dir = TempDir::new()?;

        let bb_path = get_bb_path()?;
        let mut cmd = Command::new(&bb_path);
        cmd.arg("prove")
            .arg("-v")
            .arg("--write_vk")
            .arg("-b")
            .arg(program_file.path())
            .arg("-w")
            .arg(witness_file.path())
            .arg("-o")
            .arg(output_dir.path());

        match target {
            VerifierTarget::NoirRecursive => {
                cmd.arg("--verifier_target").arg("noir-recursive");
            }
            VerifierTarget::Evm => {
                cmd.arg("--verifier_target").arg("evm");
            }
            VerifierTarget::Default => {}
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?;
            return Err(stderr.into());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            warn!("bb prove stderr: {}", stderr);
        }

        let proof_path = output_dir.path().join("proof");
        let raw_proof = std::fs::read(&proof_path)?;

        let public_inputs_path = output_dir.path().join("public_inputs");
        let public_inputs = std::fs::read(&public_inputs_path)?;

        // Concatenate: public_inputs || proof (callers split by known PI count)
        let mut result = Vec::with_capacity(public_inputs.len() + raw_proof.len());
        result.extend_from_slice(&public_inputs);
        result.extend_from_slice(&raw_proof);

        Ok(result)
    }

    fn verify(proof: &[u8], key: &[u8], target: VerifierTarget, public_inputs_len: usize) -> Result<()> {
        let mut key_file = NamedTempFile::new()?;
        key_file.write_all(key)?;
        key_file.flush()?;

        if public_inputs_len > proof.len() {
            return Err(format!(
                "public_inputs_len ({}) exceeds proof length ({})",
                public_inputs_len, proof.len()
            ).into());
        }

        let public_inputs_data = &proof[..public_inputs_len];
        let proof_data = &proof[public_inputs_len..];

        let mut proof_file = NamedTempFile::new()?;
        proof_file.write_all(proof_data)?;
        proof_file.flush()?;

        let mut public_inputs_file = NamedTempFile::new()?;
        public_inputs_file.write_all(public_inputs_data)?;
        public_inputs_file.flush()?;

        let bb_path = get_bb_path()?;

        let mut cmd = Command::new(&bb_path);
        cmd.arg("verify")
            .arg("-v")
            .arg("-k")
            .arg(key_file.path())
            .arg("-p")
            .arg(proof_file.path())
            .arg("-i")
            .arg(public_inputs_file.path());

        match target {
            VerifierTarget::NoirRecursive => {
                cmd.arg("--verifier_target").arg("noir-recursive");
            }
            VerifierTarget::Evm => {
                cmd.arg("--verifier_target").arg("evm");
            }
            VerifierTarget::Default => {}
        }

        let output = cmd.output()?;
        info!("output {:?}", output);

        if !output.status.success() {
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
    use std::fs::{self, copy, remove_file};

    use temp_env::with_var;

    #[test]
    fn test_verify_bb_does_not_exist() {
        let nonexistent = PathBuf::from("/some/other/path/bb");
        assert!(!verify_bb_executable(&nonexistent));
    }

    #[test]
    fn test_verify_bb_does_not_execute() {
        let temp_file = NamedTempFile::new().expect("Failed to create temp file");
        let path = temp_file.path().to_path_buf();

        // Write some content but don't make it executable
        fs::write(&path, "not an executable").expect("Failed to write");

        assert!(!verify_bb_executable(&path));
    }

    #[test]
    fn test_bb_in_system_path() {
        let which_result = Command::new("which").arg("bb").output();

        match which_result {
            Ok(output) if output.status.success() => {
                // bb is in PATH, get_bb_path should succeed
                let result = get_bb_path();
                assert!(
                    result.is_ok(),
                    "bb is in PATH but get_bb_path failed: {:?}",
                    result.err()
                );

                let path = result.unwrap();
                assert!(
                    verify_bb_executable(&path),
                    "Found bb at {path:?} but it's not executable"
                );
                println!("HOME {}", path.display());
            }
            _ => {
                // bb is not in PATH, skip this test. Other tests will fail though
                eprintln!("Skipping test: bb not found in system PATH");
            }
        }
    }

    #[test]
    fn test_bb_in_local_dir() {
        let which_result = Command::new("which").arg("bb").output();

        match which_result {
            Ok(output) if output.status.success() => {
                // reset PATH
                let bb_bin = String::from_utf8_lossy(&output.stdout)
                    .into_owned()
                    .replace('\n', "");
                let existing = PathBuf::from(&bb_bin);
                with_var("HOME", Some("/tmp/bin"), || {
                    // doing similar modifications as in code because
                    // there is straight forward way to test this flow in cargo test env
                    let new = std::env::current_exe()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join("bb");
                    let _ = copy(existing, &new);
                    let result = get_bb_path();
                    assert!(
                        result.is_ok(),
                        "bb is in PATH but get_bb_path failed: {:?}",
                        result.err()
                    );

                    let path = result.unwrap();
                    assert!(
                        verify_bb_executable(&path),
                        "Found bb at {path:?} but it's not executable"
                    );
                    println!("Local {}", path.display());
                    let _ = remove_file(new);
                });
            }
            _ => {
                // create symlink
                eprintln!("Skipping test: bb not found in system PATH");
            }
        }
    }

    #[test]
    fn test_bb_in_bb_path_env() {
        with_var("HOME", Some("/tmp/bin"), || {
            with_var("BB_PATH", Some("/root/.bb"), || {
                let result = get_bb_path();
                assert!(result.is_ok(), "bb is not in BB_PATH: {:?}", result.err());

                let path = result.unwrap();
                assert!(
                    verify_bb_executable(&path),
                    "Found bb at {path:?} but it's not executable"
                );
                println!("BB_PATH {}", path.display());
            })
        });
    }
}
