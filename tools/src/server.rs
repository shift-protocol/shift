use cancellation::*;
use colored::*;
use ctrlc;
use pty::fork::Fork;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::{api, Client, ClientEvent};
use toffee::{
    MessageOutput, MessageReader, MessageWriter, TransportWriter, CLIENT_TRANSPORT,
    SERVER_TRANSPORT,
};

struct App<'a> {
    master: pty::fork::Master,
    name: String,
    client: Arc<Mutex<Client<'a>>>,
    stopped: bool,
    old_mode: termios::Termios,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String) -> Self {
        let fork = Box::leak(Box::new(Fork::from_ptmx().unwrap()));

        if fork.is_child().ok().is_some() {
            let mut args = std::env::args();
            args.next();
            let _ = Command::new(args.next().expect("No program name"))
                .args(args)
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
            name,
            client: Arc::new(Mutex::new(Client::new(MessageWriter::new(
                TransportWriter::new(SERVER_TRANSPORT, Box::new(master.clone())),
            )))),
            stopped: false,
            old_mode,
            cancellation_token_source: CancellationTokenSource::new(),
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
                        // let result = self.client.lock().unwrap().reject_transfer().unwrap();
                    }
                    ClientEvent::InboundFileOpening(_, file) => {
                        println!(
                            "[{}]: {}: {:?}",
                            self.name,
                            "Inbound file open".green(),
                            file
                        );
                        client
                            .confirm_file_opened(api::FileOpened { continue_from: 0 })
                            .unwrap();
                    }
                    ClientEvent::TransferClosed => (),
                    ClientEvent::FileClosed(_) => (),
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
    App::new("server".to_string()).run();

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
