use std::path::PathBuf;

use linkup_local_server::StateStore;

use crate::{Result, state::state_file_path};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    certs_dir: String,
}

pub async fn server(args: &Args) -> Result<()> {
    let state_store = StateStore::load(state_file_path())?;

    let https_certs_dir = PathBuf::from(&args.certs_dir);

    linkup_local_server::start(&https_certs_dir, state_store).await?;

    Ok(())
}
