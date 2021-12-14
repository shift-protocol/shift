use cancellation::*;
use clap::{self, AppSettings, Parser, Subcommand};
use ctrlc;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use toffee::pty::{enable_raw_mode, restore_mode};

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
        }
    }

    pub fn run(&mut self) {
        let token = self.cancellation_token_source.token().clone();
        let client = self.client.clone();
        let mut stdin = io::stdin();
        client.lock().unwrap().run(true, &mut stdin, self, &token);
    }
}

impl<'a> FileClientDelegate<'a> for App<'a> {
    fn on_idle(&mut self, client: &mut FileClient<'a>) {
        if self.send_mode {
            if self.paths.len() == 0 {
                client.disconnect();
                std::process::exit(0);
            }
            let path_str = self.paths.remove(0);
            let path = Path::new(&path_str).canonicalize().unwrap();
            client.send_file(&path);
        } else {
            if self.remaining_receives > 0 {
                self.remaining_receives -= 1;
                client.receive();
            } else {
                client.disconnect();
                std::process::exit(0);
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let old_mode = enable_raw_mode(0);
    ctrlc::set_handler(move || {
        restore_mode(0, old_mode);
        std::process::exit(1);
    })
    .unwrap();

    App::new(cli).run();

    restore_mode(0, old_mode);
}
