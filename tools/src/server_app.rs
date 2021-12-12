use cancellation::*;
use clap::{self, Parser};
use colored::*;
use ctrlc;
use path_clean::PathClean;
use pty::fork::Fork;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::{api, Client, ClientEvent};
use toffee::{
    MessageOutput, MessageReader, MessageWriter, TransportWriter, CLIENT_TRANSPORT,
    SERVER_TRANSPORT,
};

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
    name: String,
    work_dir: String,
    client: Arc<Mutex<Client<'a>>>,
    stopped: bool,
    old_mode: termios::Termios,
    cancellation_token_source: CancellationTokenSource,

    open_file: Option<File>,
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
            name: "server".to_string(),
            work_dir: args.directory,
            client: Arc::new(Mutex::new(Client::new(MessageWriter::new(
                TransportWriter::new(SERVER_TRANSPORT, Box::new(master.clone())),
            )))),
            stopped: false,
            old_mode,
            cancellation_token_source: CancellationTokenSource::new(),
            open_file: None,
            current_inbound_transfer: None,
        };

        return _self;
    }

    pub fn run(&mut self) {
        let token = self.cancellation_token_source.token().clone();
        self.feed_input(&token).unwrap();
    }

    fn feed_input(&mut self, ct: &CancellationToken) -> Result<(), OperationCanceled> {
        // let client = self.client.clone();
        let mut stdout = io::stdout();
        let mut master = self.master.clone();

        for msg in MessageReader::new(CLIENT_TRANSPORT).feed_from(&mut master, ct) {
            match msg {
                MessageOutput::Passthrough(data) => {
                    stdout.write(&data).unwrap();
                    stdout.flush().unwrap();
                }
                MessageOutput::Message(msg) => {
                    {
                        self.client.lock().unwrap().feed_message(msg).unwrap();
                    }
                    self.process_events();
                }
            }
        }

        Ok(())
    }

    fn process_events(&mut self) {
        let lock = self.client.clone();

        let mut client = lock.lock().unwrap();
        let events = client.take_events();
        let num_events = events.len();
        {
            for event in events {
                match event {
                    ClientEvent::Connected => (),
                    ClientEvent::Disconnected => {
                        println!("[{}]: {}", self.name, "Disconnected by client".green());
                        self.stop();
                    }
                    ClientEvent::InboundTransferOffered(request) => {
                        println!(
                            "[{}]: {}: {:?}",
                            self.name,
                            "Inbound transfer offer".green(),
                            request
                        );
                        client.accept_transfer().unwrap();
                        self.current_inbound_transfer = Some(request);
                    }
                    ClientEvent::InboundFileOpening(_, file) => {
                        let transfer = self.current_inbound_transfer.clone();
                        let rel_path =
                            Path::new(&transfer.and_then(|x| x.file_info).map(|x| x.name).unwrap())
                                .join(file.file_info.unwrap().name)
                                .clean();
                        let path = Path::new(&self.work_dir).join(rel_path);

                        println!(
                            "[{}]: {}: {:?}",
                            self.name,
                            "Inbound file open".green(),
                            path
                        );

                        let mut file;
                        if path.exists() {
                            file = OpenOptions::new().append(true).open(path.clone()).unwrap();
                        } else {
                            file = File::create(path.clone()).unwrap();
                        }
                        let position = file.seek(SeekFrom::End(0)).unwrap();

                        client
                            .confirm_file_opened(api::FileOpened {
                                continue_from: position,
                            })
                            .unwrap();

                        self.open_file = Some(file);
                    }
                    ClientEvent::Chunk(chunk) => {
                        println!(
                            "[{}]: {}: {:?}",
                            self.name,
                            "Got chunk".green(),
                            chunk.data.len(),
                        );
                        if let Some(file) = &mut self.open_file {
                            file.write(&chunk.data).unwrap();
                        }
                    }
                    ClientEvent::TransferClosed => {
                        self.current_inbound_transfer = None;
                    }
                    ClientEvent::FileClosed(_) => {
                        self.open_file = None;
                    }
                    ClientEvent::OutboundTransferOffered(_) => {
                        let path = Path::new(&self.work_dir);
                        let item = path.read_dir().unwrap().next();
                        match item {
                            Some(item) => {
                                let item = item.unwrap();
                                self.open_file = Some(File::open(item.path()).unwrap());
                            },
                            None => {
                                println!("[{}]: {}", self.name, "No files to send".green());
                                client.disconnect().unwrap();
                            }
                        }
                    },
                    other => {
                        println!("[{}]: {}: {:?}", self.name, "Unknown event".red(), other);
                        self.stop();
                    }
                };
            }
        }

        drop(client);
        if num_events > 0 {
            self.process_events();
        }
    }

    fn stop(&mut self) {
        println!("Stopping");
        self.cancellation_token_source.cancel();
        self.stopped = true;
        restore_mode(0, self.old_mode);
    }
}

fn main() {
    let cli = Cli::parse();
    App::new(cli).run();

    // let mut message_writer =
    //     MessageWriter::new(TransportWriter::new(SERVER_TRANSPORT, Box::new(master)));

    // let cancellation_token_source = CancellationTokenSource::new();

    // let mut message_reader = MessageReader::new(CLIENT_TRANSPORT);

    // let mut stdout = io::stdout();

    // for msg in message_reader.feed_from(&mut master, &cancellation_token_source.token().clone()) {
    //     println!("{:?}", msg);
    //     match msg {
    //         MessageOutput::Passthrough(data) => {
    //             stdout.write(&data).unwrap();
    //         },
    //         MessageOutput::Message(data) => {

    //         }
    //     }
    // }
}
