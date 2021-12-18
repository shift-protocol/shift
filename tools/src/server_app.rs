use cancellation::*;
use clap::{self, Parser};
use colored::*;
use ctrlc;
use path_clean::PathClean;
use pty::fork::Fork;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use shift::api;
use shift::pty::{enable_raw_mode, restore_mode};

mod file_client;
use file_client::{FileClient, FileClientDelegate};

#[derive(Parser, Debug)]
#[clap(version)]
struct Cli {
    #[clap(short, long)]
    directory: String,

    #[clap(multiple_values = true)]
    args: Vec<String>,
}

struct App<'a> {
    master: pty::fork::Master,
    work_dir: String,
    client: Arc<Mutex<FileClient<'a>>>,
    old_mode: termios::Termios,
    cancellation_token_source: CancellationTokenSource,

    current_inbound_transfer: Option<api::SendRequest>,
}

impl<'a> App<'a> {
    pub fn new(args: Cli) -> Self {
        let fork = Box::leak(Box::new(Fork::from_ptmx().unwrap()));

        if fork.is_child().ok().is_some() {
            println!("Starting {}", args.args[0]);
            let _ = Command::new(&args.args[0])
                .args(&args.args[1..])
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap()
                .wait();
            std::process::exit(0);
        }

        let master = fork.is_parent().ok().unwrap();
        let old_mode = enable_raw_mode(0);

        ctrlc::set_handler(move || {
            restore_mode(0, old_mode);
            std::process::exit(1);
        })
        .unwrap();

        thread::spawn({
            let mut master = master.clone();
            let mut stdin = io::stdin();
            move || {
                let mut buf = [0; 1024];
                loop {
                    let size = stdin.read(&mut buf).expect("read error");
                    if size == 0 {
                        break;
                    }
                    if master.write(&buf[..size]).is_err() {
                        break;
                    }
                }
            }
        });

        let mut _self = Self {
            master: master.clone(),
            work_dir: args.directory,
            client: Arc::new(Mutex::new(FileClient::new(
                "server".to_string(),
                Box::new(master.clone()),
                Some(Box::new(io::stdout())),
            ))),
            old_mode,
            cancellation_token_source: CancellationTokenSource::new(),
            current_inbound_transfer: None,
        };

        return _self;
    }

    pub fn run(&mut self) {
        let token = self.cancellation_token_source.token().clone();
        let client = self.client.clone();
        let mut master = self.master.clone();
        client.lock().unwrap().run(false, &mut master, self, &token);
    }

    fn stop(&mut self) {
        println!("Stopping");
        self.cancellation_token_source.cancel();
        restore_mode(0, self.old_mode);
    }
}

impl<'a> FileClientDelegate<'a> for App<'a> {
    fn on_inbound_transfer_request(&mut self, request: &api::SendRequest) -> bool {
        println!("[server]: inbound request {:?}", request);
        self.current_inbound_transfer = Some(request.clone());
        return true;
    }

    fn on_inbound_transfer_file(&mut self, file: &api::OpenFile) -> Option<PathBuf> {
        let transfer = self.current_inbound_transfer.clone();
        let rel_path = Path::new(&transfer.and_then(|x| x.file_info).map(|x| x.name).unwrap())
            .join(file.file_info.clone().unwrap().name)
            .clean();
        let path = PathBuf::from(&self.work_dir).join(rel_path);
        println!("[server]: {}: {:?}", "Inbound file open".green(), path);
        return Some(path);
    }

    fn on_outbound_transfer_request(&mut self, _request: &api::ReceiveRequest, client: &mut FileClient<'a>) {
        let path = Path::new(&self.work_dir);
        let item = path.read_dir().unwrap().next();
        match item {
            Some(item) => {
                let item = item.unwrap();
                client.send_file(&item.path(), Box::new(|_, _, _| {}));
            }
            None => {
                println!("[server]: {}", "No files to send".green());
                client.disconnect();
            }
        }
    }

    fn on_transfer_closed(&mut self) {
        self.current_inbound_transfer = None;
    }

    fn on_disconnect(&mut self) {
        self.stop();
    }
}

fn main() {
    let cli = Cli::parse();
    App::new(cli).run();
}
