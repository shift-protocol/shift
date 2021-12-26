use anyhow::Result;

#[cfg(target_family = "unix")]
mod host;

#[cfg(target_family = "unix")]
fn main() -> Result<()> {
    use clap::StructOpt;

    let cli = host::Cli::parse();
    host::App::new(cli)?.run()?;
    Ok(())
}

#[cfg(target_family = "windows")]
fn main() {}
