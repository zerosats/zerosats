use std::path::PathBuf;
use std::time::Duration;

use cli::NodeClient;
use eyre::Result;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use tracing::{error, info, instrument, warn};
use zk_primitives::InputNote;

use crate::actions;
use crate::config::Args;

struct PlayerWallet {
    name: String,
    client: NodeClient,
}

/// Ring buffer of recently spent notes for double-spend fault injection
struct SpentNoteLog {
    notes: Vec<InputNote>,
}

impl SpentNoteLog {
    fn new() -> Self {
        Self { notes: Vec::new() }
    }

    fn push(&mut self, note: InputNote) {
        if self.notes.len() >= 20 {
            self.notes.remove(0);
        }
        self.notes.push(note);
    }

    fn random(&self, rng: &mut ChaCha8Rng) -> Option<&InputNote> {
        if self.notes.is_empty() {
            return None;
        }
        let idx = rng.gen_range(0..self.notes.len());
        Some(&self.notes[idx])
    }
}

#[derive(Debug, Clone, Copy)]
enum Action {
    Mint,
    SpendReceive,
    SelfSpend,
    Burn,
    FaultGarbage,
    FaultDoubleSpend,
}

pub struct Player {
    wallets: Vec<PlayerWallet>,
    spent_log: SpentNoteLog,
    config: Args,
    rng: ChaCha8Rng,
    action_weights: Vec<(Action, u32)>,
    round: u64,
}

impl Player {
    pub fn new(config: Args) -> Result<Self> {
        let actual_seed = if config.seed == 0 {
            rand::random()
        } else {
            config.seed
        };
        info!(seed = actual_seed, "RNG initialized — use --seed={actual_seed} to replay");
        let rng = ChaCha8Rng::seed_from_u64(actual_seed);

        std::fs::create_dir_all(&config.wallet_dir)?;

        let mut wallets = Vec::new();
        for i in 0..config.wallet_count {
            let name = format!("player-{i}");
            let client = NodeClient::builder()
                .name(&name)
                .host(&config.host)
                .port(config.port)
                .timeout_secs(60)
                .wallet_dir(PathBuf::from(&config.wallet_dir))
                .build(config.chain_id, false, true)?;

            wallets.push(PlayerWallet { name, client });
        }

        let fault_half = config.weight_fault / 2;
        let action_weights = vec![
            (Action::Mint, config.weight_mint),
            (Action::SpendReceive, config.weight_spend),
            (Action::SelfSpend, config.weight_self_spend),
            (Action::Burn, config.weight_burn),
            (Action::FaultGarbage, fault_half),
            (Action::FaultDoubleSpend, config.weight_fault - fault_half),
        ];

        Ok(Self {
            wallets,
            spent_log: SpentNoteLog::new(),
            config,
            rng,
            action_weights,
            round: 0,
        })
    }

    fn pick_action(&mut self) -> Action {
        let total: u32 = self.action_weights.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return Action::Mint;
        }
        let mut roll = self.rng.gen_range(0..total);
        for (action, weight) in &self.action_weights {
            if roll < *weight {
                return *action;
            }
            roll -= weight;
        }
        Action::Mint
    }

    fn pick_wallet(&mut self) -> usize {
        self.rng.gen_range(0..self.wallets.len())
    }

    fn pick_other_wallet(&mut self, exclude: usize) -> usize {
        if self.wallets.len() <= 1 {
            return exclude;
        }
        loop {
            let idx = self.pick_wallet();
            if idx != exclude {
                return idx;
            }
        }
    }

    fn pick_amount(&mut self) -> u64 {
        self.rng
            .gen_range(self.config.min_amount..=self.config.max_amount)
    }

    fn pick_delay(&mut self) -> Duration {
        let ms = self
            .rng
            .gen_range(self.config.min_delay_ms..=self.config.max_delay_ms);
        Duration::from_millis(ms)
    }

    fn wallet_balance(&self, idx: usize) -> u64 {
        self.wallets[idx].client.get_wallet().balance
    }

    fn find_funded_wallet(&mut self, min_balance: u64) -> Option<usize> {
        let candidates: Vec<usize> = (0..self.wallets.len())
            .filter(|&i| self.wallet_balance(i) >= min_balance)
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let idx = self.rng.gen_range(0..candidates.len());
        Some(candidates[idx])
    }

    #[instrument(skip(self), name = "autoplayer")]
    pub async fn run(&mut self) -> Result<()> {
        info!(wallet_count = self.wallets.len(), "autoplayer starting");

        // Initial funding
        for i in 0..self.wallets.len() {
            let amount = self.pick_amount() * 10;
            let name = self.wallets[i].name.clone();
            info!(wallet = %name, amount, "initial funding mint");
            if let Err(e) = actions::do_mint(
                &mut self.wallets[i].client,
                &name,
                amount,
                &self.config,
            )
            .await
            {
                error!(wallet = %name, error = %e, "initial mint failed — will retry in loop");
            }
        }

        loop {
            self.round += 1;
            let action = self.pick_action();

            info!(round = self.round, action = ?action, "--- round start ---");

            let result = match action {
                Action::Mint => self.execute_mint().await,
                Action::SpendReceive => self.execute_spend_receive().await,
                Action::SelfSpend => self.execute_self_spend().await,
                Action::Burn => self.execute_burn().await,
                Action::FaultGarbage => self.execute_fault_garbage().await,
                Action::FaultDoubleSpend => self.execute_fault_double_spend().await,
            };

            match result {
                Ok(()) => info!(round = self.round, action = ?action, "round succeeded"),
                Err(e) => warn!(round = self.round, action = ?action, error = %e, "round failed"),
            }

            let delay = self.pick_delay();
            info!(delay_ms = delay.as_millis(), "sleeping");
            tokio::time::sleep(delay).await;
        }
    }

    async fn execute_mint(&mut self) -> Result<()> {
        let idx = self.pick_wallet();
        let amount = self.pick_amount();
        let name = self.wallets[idx].name.clone();
        actions::do_mint(&mut self.wallets[idx].client, &name, amount, &self.config).await
    }

    /// Spend from one wallet, receive in another — always paired, closes the note loop
    async fn execute_spend_receive(&mut self) -> Result<()> {
        let amount = self.pick_amount();
        let sender_idx = match self.find_funded_wallet(amount) {
            Some(idx) => idx,
            None => {
                info!("no funded wallet for spend, minting instead");
                return self.execute_mint().await;
            }
        };
        let receiver_idx = self.pick_other_wallet(sender_idx);

        let sender_name = self.wallets[sender_idx].name.clone();
        let receiver_name = self.wallets[receiver_idx].name.clone();

        // Spend produces a note
        let transfer_note = actions::do_spend(
            &mut self.wallets[sender_idx].client,
            &sender_name,
            amount,
        )
        .await?;

        // Log for fault injection
        self.spent_log.push(transfer_note.clone());

        // Receive consumes it
        actions::do_receive(
            &mut self.wallets[receiver_idx].client,
            &receiver_name,
            &transfer_note,
        )
        .await?;

        Ok(())
    }

    /// Self-spend: spend and receive in the same wallet (UTXO churn)
    async fn execute_self_spend(&mut self) -> Result<()> {
        let amount = self.pick_amount();
        let idx = match self.find_funded_wallet(amount) {
            Some(idx) => idx,
            None => {
                info!("no funded wallet for self-spend, minting instead");
                return self.execute_mint().await;
            }
        };

        let name = self.wallets[idx].name.clone();

        let transfer_note =
            actions::do_spend(&mut self.wallets[idx].client, &name, amount).await?;

        self.spent_log.push(transfer_note.clone());

        actions::do_receive(&mut self.wallets[idx].client, &name, &transfer_note).await?;

        Ok(())
    }

    async fn execute_burn(&mut self) -> Result<()> {
        let amount = self.pick_amount();
        let idx = match self.find_funded_wallet(amount) {
            Some(idx) => idx,
            None => {
                info!("no funded wallet for burn, minting instead");
                return self.execute_mint().await;
            }
        };

        let name = self.wallets[idx].name.clone();
        // Burn to Hardhat account 0
        let dummy_evm_addr = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
        actions::do_burn(&mut self.wallets[idx].client, &name, amount, dummy_evm_addr).await
    }

    async fn execute_fault_garbage(&mut self) -> Result<()> {
        let idx = self.pick_wallet();
        actions::do_fault_garbage_proof(&mut self.wallets[idx].client).await
    }

    async fn execute_fault_double_spend(&mut self) -> Result<()> {
        match self.spent_log.random(&mut self.rng).cloned() {
            Some(note) => {
                let idx = self.pick_wallet();
                let name = self.wallets[idx].name.clone();
                actions::do_fault_double_spend(&mut self.wallets[idx].client, &name, &note).await
            }
            None => {
                info!("no spent notes logged yet, skipping double-spend fault");
                Ok(())
            }
        }
    }
}
