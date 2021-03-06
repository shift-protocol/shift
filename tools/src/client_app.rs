use anyhow::{anyhow, Result};
use cancellation::*;
use clap::{self, AppSettings, Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use path_clean::PathClean;
use shift::pty::{enable_raw_mode, restore_mode};
use shift::{api, MessageWriter, ShiftClient, TransportWriter, TRANSPORT};
use shift_fileclient::{ShiftFileClient, ShiftFileClientDelegate};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Subcommand, Debug, PartialEq)]
enum Commands {
    /// Send files or directories
    Send {
        #[clap(multiple_values = true)]
        paths: Vec<String>,
    },
    /// Receive files or directories
    Receive {
        #[clap(multiple_values = true)]
        paths: Vec<String>,
    },
}

#[derive(Parser, Debug)]
#[clap(name = "shift")]
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

    client: Arc<Mutex<ShiftFileClient<'a>>>,
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
            client: Arc::new(Mutex::new(ShiftFileClient::new(
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

impl<'a> ShiftFileClientDelegate<'a> for App<'a> {
    fn on_idle(&mut self, client: &mut ShiftFileClient<'a>) -> Result<()> {
        if self.send_mode {
            if self.paths.is_empty() {
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
        } else if self.remaining_receives > 0 {
            self.remaining_receives -= 1;
            client.receive()?;
        } else {
            client.disconnect()?;
            std::process::exit(0);
        }
        Ok(())
    }

    fn on_inbound_transfer_request(&mut self, request: &api::SendRequest) -> bool {
        if self.send_mode {
            return false;
        }
        self.current_inbound_transfer = Some(request.clone());
        true
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
        Ok(Some(rel_path))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let old_mode = enable_raw_mode(0).ok();
    let abort = move || {
        if let Some(old_mode) = old_mode {
            restore_mode(0, old_mode).expect("Failed to restore TTY mode");
        }
        let _ = ShiftClient::new(MessageWriter::new(TransportWriter::new(
            TRANSPORT,
            Box::new(std::io::stdout()),
        )))
        .disconnect();
    };

    ctrlc::set_handler(move || {
        abort();
        std::process::exit(1);
    })?;

    App::new(cli).run().unwrap_or_else(|e| {
        abort();
        panic!("{}", e);
    });

    if let Some(old_mode) = old_mode {
        restore_mode(0, old_mode)?;
    }
    Ok(())
}
