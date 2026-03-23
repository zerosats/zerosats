use barretenberg::Prove;
use cli::NodeClient;
use eyre::Result;
use tracing::{error, info, instrument, warn};

use crate::config::Args;

/// Mint new tokens into a wallet from the Citrea bridge.
/// Mirrors handle_mint in cli/src/main.rs:496-552.
#[instrument(skip_all, fields(wallet = %wallet_name, amount))]
pub async fn do_mint(
    client: &mut NodeClient,
    wallet_name: &str,
    amount: u64,
    config: &Args,
) -> Result<()> {
    info!(amount, "minting tokens");

    let (prepared_wallet, utxo) = client.get_wallet().prepare_mint(amount, "WCBTC")?;
    let snark = utxo.prove().map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    // Bridge call on Citrea — deposits tokens into the rollup contract
    client
        .admin_mint(
            &config.evm_rpc_url,
            config.chain_id,
            &config.evm_secret,
            &config.rollup_contract,
            &utxo.output_notes[0],
            &snark,
        )
        .await?;

    let resp = client.transaction(&snark).await?;
    info!(txn_hash = %resp.txn_hash, height = resp.height.0, "mint successful");

    prepared_wallet.save()?;
    client.replace_wallet(prepared_wallet);
    Ok(())
}

/// Spend tokens from sender, producing a note for receiver.
/// Returns the InputNote that the receiver should consume via do_receive.
/// Mirrors handle_note_spend in cli/src/main.rs:271-325.
#[instrument(skip_all, fields(sender = %sender_name, amount))]
pub async fn do_spend(
    sender_client: &mut NodeClient,
    sender_name: &str,
    amount: u64,
) -> Result<zk_primitives::InputNote> {
    info!(amount, "spending tokens");

    let (wallet_with_note, transfer_note) = sender_client
        .get_wallet()
        .prepare_receive_note(amount, "WCBTC");
    let (prepared_wallet, utxo) = wallet_with_note.prepare_spend_to(&transfer_note.note)?;
    let snark = utxo.prove().map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    let resp = sender_client.transaction(&snark).await?;
    info!(txn_hash = %resp.txn_hash, height = resp.height.0, "spend successful");

    prepared_wallet.save()?;
    sender_client.replace_wallet(prepared_wallet);

    Ok(transfer_note)
}

/// Receive a note that was produced by do_spend.
/// Mirrors handle_receive in cli/src/main.rs:384-478.
#[instrument(skip_all, fields(receiver = %receiver_name))]
pub async fn do_receive(
    receiver_client: &mut NodeClient,
    receiver_name: &str,
    input_note: &zk_primitives::InputNote,
) -> Result<()> {
    info!("receiving note");

    let (prepared_wallet, utxo) = receiver_client.get_wallet().prepare_receive(input_note)?;
    let snark = utxo.prove().map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    let resp = receiver_client.transaction(&snark).await?;
    info!(txn_hash = %resp.txn_hash, height = resp.height.0, "receive successful");

    prepared_wallet.save()?;
    receiver_client.replace_wallet(prepared_wallet);
    Ok(())
}

/// Burn tokens back to an EVM address on Citrea.
/// This is a 3-step process (mirrors handle_burn in cli/src/main.rs:554-623):
/// 1. Create burner note via prepare_receive_note + prepare_spend_to, prove, submit
/// 2. Import the burner note via prepare_import_note
/// 3. Execute burn via prepare_burn with EVM address, prove, submit
#[instrument(skip_all, fields(wallet = %wallet_name, amount))]
pub async fn do_burn(
    client: &mut NodeClient,
    wallet_name: &str,
    amount: u64,
    evm_address: &str,
) -> Result<()> {
    use contracts::util::convert_h160_to_element;
    use std::str::FromStr;
    use web3::types::H160;

    info!(amount, evm_address, "burning tokens (3-step)");

    // Step 1: Create burner note and transfer to self
    let (wallet_with_burner, burner_note) =
        client.get_wallet().prepare_receive_note(amount, "WCBTC");
    let (wallet_after_transfer, burner_utxo) =
        wallet_with_burner.prepare_spend_to(&burner_note.note)?;
    let snark = burner_utxo
        .prove()
        .map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    let resp = client.transaction(&snark).await?;
    info!(txn_hash = %resp.txn_hash, "burn step 1: transfer to burner note");

    wallet_after_transfer.save()?;
    client.replace_wallet(wallet_after_transfer);

    // Step 2: Import the burner note
    let (wallet_with_import, _) = client
        .get_wallet()
        .prepare_import_note(&burner_note.note)?;
    wallet_with_import.save()?;
    client.replace_wallet(wallet_with_import);

    // Step 3: Execute burn
    let addr = H160::from_str(evm_address)?;
    let evm_element = convert_h160_to_element(&addr);

    let (wallet_after_burn, burn_utxo) = client
        .get_wallet()
        .prepare_burn(&burner_note, &evm_element)?;
    let snark = burn_utxo
        .prove()
        .map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    let resp = client.transaction(&snark).await?;
    info!(txn_hash = %resp.txn_hash, height = resp.height.0, "burn step 3: burn successful");

    wallet_after_burn.save()?;
    client.replace_wallet(wallet_after_burn);
    Ok(())
}

/// Fault injection: submit a corrupted proof
#[instrument(skip_all)]
pub async fn do_fault_garbage_proof(client: &mut NodeClient) -> Result<()> {
    warn!("FAULT: submitting garbage proof");

    // Create a valid mint proof, then corrupt it
    let wallet = cli::Wallet::random(5655, Some("fault-temp".into()));
    let (_, utxo) = wallet.prepare_mint(1000, "WCBTC")?;
    let mut snark = utxo.prove().map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;

    // Corrupt the proof by flipping bits in public inputs
    if let Some(msg) = snark.public_inputs.messages.first_mut() {
        *msg = element::Element::new(999999);
    }

    match client.transaction(&snark).await {
        Ok(_) => {
            error!("FAULT: garbage proof was ACCEPTED — this is a bug!");
            Err(eyre::eyre!("node accepted invalid proof"))
        }
        Err(e) => {
            info!("FAULT: garbage proof correctly rejected: {e}");
            Ok(())
        }
    }
}

/// Fault injection: double-spend a note
#[instrument(skip_all, fields(wallet = %wallet_name))]
pub async fn do_fault_double_spend(
    client: &mut NodeClient,
    wallet_name: &str,
    spent_note: &zk_primitives::InputNote,
) -> Result<()> {
    warn!("FAULT: attempting double-spend");

    match client.get_wallet().prepare_spend_to(&spent_note.note) {
        Ok((_, utxo)) => {
            let snark = utxo.prove().map_err(|e| eyre::eyre!("prove failed: {e:?}"))?;
            match client.transaction(&snark).await {
                Ok(_) => {
                    error!("FAULT: double-spend was ACCEPTED — this is a bug!");
                    Err(eyre::eyre!("node accepted double-spend"))
                }
                Err(e) => {
                    info!("FAULT: double-spend correctly rejected: {e}");
                    Ok(())
                }
            }
        }
        Err(e) => {
            info!("FAULT: wallet correctly refused double-spend: {e}");
            Ok(())
        }
    }
}
