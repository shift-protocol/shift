use cancellation::*;
use colored::*;
use ctrlc;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use std::cell::RefCell;
use toffee::api;
use toffee::helpers::send_file;
use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::{
    Client, ClientEvent, MessageOutput, MessageReader, MessageWriter, TransportWriter,
    CLIENT_TRANSPORT, SERVER_TRANSPORT,
};
use clap::{self, Parser, AppSettings, Subcommand};

mod file_client;
use file_client::{FileClient, FileClientDelegate};

#[derive(Subcommand, Debug, PartialEq)]
enum Commands {
    Send {
        #[clap(multiple_values=true)]
        paths: Vec<String>,
    },
    Receive {
        #[clap(multiple_values=true)]
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
    name: String,
    send_mode: bool,
    remaining_receives: u32,

    paths: Vec<String>,

    client: Rc<RefCell<FileClient<'a>>>,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String, args: Cli) -> Self {
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
            name,
            send_mode,
            paths: _paths,
            remaining_receives: 1,
            client: Rc::new(RefCell::new(FileClient::new("client".to_string()))),
            cancellation_token_source: CancellationTokenSource::new(),
        }
    }

    pub fn run(&mut self) {
        let token = self.cancellation_token_source.token().clone();
        let client = self.client.clone();
        client.borrow_mut().run(true, self, &token);
    }

    fn stop(&mut self) {
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
                client.disconnect();
                std::process::exit(0);
            } else {
                self.stop();
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

    App::new("client".to_string(), cli).run();

    restore_mode(0, old_mode);
}
