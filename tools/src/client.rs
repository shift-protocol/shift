use cancellation::*;
use colored::*;
use ctrlc;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use toffee::api;
use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::{
    Client, ClientEvent, MessageOutput, MessageReader, MessageWriter, TransportWriter,
    CLIENT_TRANSPORT, SERVER_TRANSPORT,
};
use clap::{self, Parser, AppSettings, Subcommand};

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
    current_path: Option<String>,

    client: Arc<Mutex<Client<'a>>>,
    stopped: bool,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String, args: Cli) -> Self {
        let stdout = io::stdout();

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
            current_path: None,
            client: Arc::new(Mutex::new(Client::new(MessageWriter::new(
                TransportWriter::new(CLIENT_TRANSPORT, Box::new(stdout)),
            )))),
            stopped: false,
            cancellation_token_source: CancellationTokenSource::new(),
        }
    }

    pub fn run(&mut self) {
        {
            self.client.lock().unwrap().start().unwrap();
        }
        crossbeam::scope(|scope| {
            println!("[{}]: {}", self.name, "Connected".green());

            let (tx, rx) = std::sync::mpsc::channel();

            let reader_thread = scope.spawn({
                let token = self.cancellation_token_source.token().clone();
                let client = self.client.clone();
                let mut stdin = io::stdin();
                let tx = tx.clone();
                move |_| {
                    tx.send(0).unwrap();
                    let mut reader = MessageReader::new(SERVER_TRANSPORT);
                    for msg in reader.feed_from(&mut stdin, &token) {
                        if let MessageOutput::Message(msg) = msg {
                            client.lock().unwrap().feed_message(msg).unwrap();
                            tx.send(0).unwrap();
                        }
                    }
                }
            });

            loop {
                let events = { self.client.lock().unwrap().take_events() };
                if events.len() == 0 {
                    rx.recv().unwrap();
                }
                for event in events {
                    match event {
                        ClientEvent::Connected => {
                            self.on_idle();
                        }
                        ClientEvent::Disconnected => {
                            println!("[{}]: {}", self.name, "Disconnected by server".green());
                            self.stop();
                        }
                        ClientEvent::TransferAccepted() => {
                            println!("[{}]: {}", self.name, "Transfer accepted".green());
                            self.client
                                .lock()
                                .unwrap()
                                .open_file(api::OpenFile {
                                    file_info: Some(api::FileInfo {
                                        name: ".".to_string(),
                                        size: 3,
                                        mode: 0o644,
                                    }),
                                })
                                .unwrap();
                        }
                        ClientEvent::FileTransferStarted(_, response) => {
                            println!("[{}]: {}: {:?}", self.name, "File open".green(), response);
                            let path = self.current_path.clone().expect("Current file not set");
                            scope.spawn({
                                let client = self.client.clone();
                                let tx = tx.clone();
                                move |_| {
                                    send_file(client, response.continue_from, path).unwrap();
                                    tx.send(0).unwrap();
                                }
                            });
                        }
                        ClientEvent::FileClosed(_) => {
                            self.client.lock().unwrap().close_transfer().unwrap();
                        }
                        ClientEvent::TransferClosed => {
                            println!("[{}]: {}", self.name, "Transfer closed".green());
                            self.on_idle();
                        }
                        other => {
                            println!("[{}]: {}: {:?}", self.name, "Unknown event".red(), other);
                            self.stop();
                            break;
                        }
                    };
                }
            }

            reader_thread.join().unwrap();
        })
        .unwrap();
    }

    fn on_idle(&mut self) {
        if self.send_mode {
            if self.paths.len() == 0 {
                self.client.lock().unwrap().disconnect().unwrap();
                return;
            }
            let path_str = self.paths.remove(0);
            let path = Path::new(&path_str).canonicalize().unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            let mode = meta.permissions().mode();
            println!("[{}]: {}", self.name, "Requesting transfer".green());
            self.client
                .lock()
                .unwrap()
                .request_inbound_transfer(api::InboundTransferRequest {
                    file_info: Some(api::FileInfo {
                        name: path
                            .file_name()
                            .and_then(|x| x.to_str())
                            .map(|x| x.to_string())
                            .unwrap(),
                        size: meta.size(),
                        mode,
                    }),
                })
                .unwrap();

            self.current_path = Some(path_str);
        } else {
            if self.remaining_receives > 0 {
                self.remaining_receives -= 1;
                self.client.lock().unwrap().request_outbound_transfer(api::OutboundTransferRequest {
                    allow_directories: false,
                    allow_multiple: false,
                }).unwrap();
            } else {
                self.client.lock().unwrap().disconnect().unwrap();
            }
        }
    }

    fn stop(&mut self) {
        self.cancellation_token_source.cancel();
        self.stopped = true;
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

fn send_file(client: Arc<Mutex<Client>>, mut position: u64, path: String) -> std::io::Result<()> {
    const CAP: usize = 1024 * 128;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(position))?;
    let mut reader = BufReader::with_capacity(CAP, file);
    loop {
        let length = {
            let buffer = reader.fill_buf()?;
            if buffer.len() == 0 {
                break;
            }
            client
                .lock()
                .unwrap()
                .send_chunk(api::Chunk {
                    offset: position,
                    data: buffer.to_vec(),
                })
                .unwrap();
            position += buffer.len() as u64;
            buffer.len()
        };
        reader.consume(length);
    }
    client.lock().unwrap().close_file().unwrap();
    Ok(())
}
