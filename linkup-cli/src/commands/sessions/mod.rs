mod create;
mod create_preview;
mod delete;

use std::path::Path;

use clap::Subcommand;
use linkup::MachineId;

use crate::Result;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[clap(about = "Create an additional tunneled session")]
    Create(create::Args),

    #[clap(about = "Create a preview session")]
    CreatePreview(create_preview::Args),

    #[clap(about = "Delete an additional tunneled session")]
    Delete(delete::Args),
}

pub async fn sessions(args: &Args, config: Option<&Path>, machine_id: MachineId) -> Result<()> {
    match &args.command {
        Command::Create(args) => create::run(args, config, machine_id).await,
        Command::CreatePreview(args) => create_preview::run(args, config).await,
        Command::Delete(args) => delete::run(args).await,
    }
}
