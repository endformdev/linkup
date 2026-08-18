use std::path::PathBuf;

use linkup::MemoryStringStore;
use linkup_local_server::StateStore;

use crate::{
    Result,
    state::{State, state_file_path},
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    certs_dir: String,
}

pub async fn server(args: &Args) -> Result<()> {
    let state = State::load()?;
    let state_store = StateStore::load(state_file_path())?;

    let config_store = MemoryStringStore::default();
    let https_certs_dir = PathBuf::from(&args.certs_dir);

    linkup_local_server::start(
        config_store,
        &https_certs_dir,
        &state.worker_url,
        &state.worker_token,
        Some(state_store),
    )
    .await;

    Ok(())
}
