use anyhow::{anyhow, Result};
use cancellation::*;
use clap::{self, AppSettings, Parser, Subcommand};
use ctrlc;
use indicatif::{ProgressBar, ProgressStyle};
use path_clean::PathClean;
use shift::api;
use shift::pty::{enable_raw_mode, restore_mode};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

mod file_client;
use file_client::{FileClient, FileClientDelegate};

#[derive(Subcommand, Debug, PartialEq)]
enum Commands {
    Send {
        #[clap(multiple_values = true)]
        paths: Vec<String>,
    },
    Receive {
        #[clap(multiple_values = true)]
        paths: Vec<String>,
    },
}

#[derive(Parser, Debug)]
#[clap(version)]
#[clap(setting(AppSettings::SubcommandRequiredElseHelp))]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

struct App<'a> {
    send_mode: bool,
    remaining_receives: u32,

    paths: Vec<String>,

    client: Arc<Mutex<FileClient<'a>>>,
    cancellation_token_source: CancellationTokenSource,
    current_inbound_transfer: Option<api::SendRequest>,
}

impl<'a> App<'a> {
    pub fn new(args: Cli) -> Self {
        let _paths;
        let send_mode;
        match args.command {
            Commands::Send { paths } => {
                _paths = paths;
                send_mode = true;
            }
            Commands::Receive { paths } => {
                _paths = paths;
                send_mode = false;
            }
        }
        Self {
            send_mode,
            paths: _paths,
            remaining_receives: 1,
            client: Arc::new(Mutex::new(FileClient::new(
                "client".to_string(),
                Box::new(io::stdout()),
                None,
            ))),
            cancellation_token_source: CancellationTokenSource::new(),
            current_inbound_transfer: None,
        }
    }

    pub fn run(mut self) -> Result<()> {
        let token = self.cancellation_token_source.token().clone();
        let client = self.client.clone();
        let mut stdin = io::stdin();
        client
            .lock()
            .unwrap()
            .run(true, &mut stdin, &mut self, &token)?;
        Ok(())
    }
}

impl<'a> FileClientDelegate<'a> for App<'a> {
    fn on_idle(&mut self, client: &mut FileClient<'a>) -> Result<()> {
        if self.send_mode {
            if self.paths.len() == 0 {
                client.disconnect()?;
                std::process::exit(0);
            }
            let path_str = self.paths.remove(0);
            let path = Path::new(&path_str).canonicalize()?;
            let bar = ProgressBar::new(1);
            bar.set_style(ProgressStyle::default_bar()
                .template("{bar:20.cyan/blue} {wide_msg} {bytes}/{total_bytes}  ETA {eta_precise}  {bytes_per_sec:10}"));
            bar.set_message("Preparing");
            client.send(
                &path,
                Box::new(move |file, sent, total| {
                    if file.info.name != "." {
                        bar.set_message(file.info.name.clone());
                    }
                    if bar.position() == 0 {
                        bar.reset_eta();
                    }
                    bar.set_length(total);
                    bar.set_position(sent);
                    if sent == total {
                        bar.finish();
                    }
                }),
            )?;
        } else {
            if self.remaining_receives > 0 {
                self.remaining_receives -= 1;
                client.receive()?;
            } else {
                client.disconnect()?;
                std::process::exit(0);
            }
        }
        Ok(())
    }

    fn on_inbound_transfer_request(&mut self, request: &api::SendRequest) -> bool {
        if self.send_mode {
            return false;
        }
        self.current_inbound_transfer = Some(request.clone());
        return true;
    }

    fn on_inbound_transfer_file(&mut self, file: &api::OpenFile) -> Result<Option<PathBuf>> {
        if self.send_mode {
            return Ok(None);
        }
        // TODO path check
        let transfer = self
            .current_inbound_transfer
            .clone()
            .ok_or(anyhow!("No active transfer"))?;
        let transfer_info = &transfer.file_info.ok_or(anyhow!("Missing file info"))?;
        let rel_path = Path::new(&transfer_info.name)
            .join(
                file.file_info
                    .clone()
                    .ok_or(anyhow!("Missing file info in request"))?
                    .name,
            )
            .clean();
        return Ok(Some(rel_path));
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let old_mode = enable_raw_mode(0)?;
    ctrlc::set_handler(move || {
        restore_mode(0, old_mode).expect("Failed to restore TTY mode");
        std::process::exit(1);
    })?;

    App::new(cli).run()?;

    restore_mode(0, old_mode)?;
    Ok(())
}
