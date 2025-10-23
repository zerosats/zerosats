# zerosats
A privacy-preserving appchain built on top of Citrea.


cargo test rpc::transaction::burn_tx -- --test-threads=1

LOG_HARDHAT_DEPLOY_OUTPUT=1 cargo test rpc::elements::list_elements_include_spent -- --test-threads=1

RUST_LOG=debug LOG_CITREA_OUTPUT=1 LOG_HARDHAT_DEPLOY_OUTPUT=1 cargo test tests::burn_to -- --test-threads=1