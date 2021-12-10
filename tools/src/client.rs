use std::io;
use toffee::{MessageReader, MessageWriter, MessageOutput, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT, Client, ClientEvent};
use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::api;
use std::cell::RefCell;
use ctrlc;
use colored::*;
use cancellation::*;

struct App<'a> {
    name: String,
    client: RefCell<Client<'a>>,
    file_requested: bool,
    stopped: bool,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String) -> Self {
        let stdout = io::stdout();

        Self {
            name,
            client: RefCell::new(Client::new(
                MessageWriter::new(TransportWriter::new(CLIENT_TRANSPORT, Box::new(stdout)))
            )),
            file_requested: false,
            stopped: false,
            cancellation_token_source: CancellationTokenSource::new(),
        }
    }

    pub fn run(&mut self) {
        self.client.borrow_mut().start().unwrap();
        self.process_events();
        println!("[{}]: {}", self.name, "Connected".green());
        let token = self.cancellation_token_source.token().clone();
        let _ = self.feed_input(&token);
    }

    fn feed_input(&mut self, ct: &CancellationToken) -> Result<(), OperationCanceled> {
        let mut stdin = io::stdin();

        for msg in MessageReader::new(SERVER_TRANSPORT).feed_from(&mut stdin, ct) {
            if let MessageOutput::Message(msg) = msg {
                self.client.borrow_mut().feed_message(msg).unwrap();
                self.process_events();
            }
        }

        Ok(())
    }

    fn process_events(&mut self) {
        let events = self.client.borrow_mut().take_events();
        for event in events {
            match event {
                ClientEvent::Connected => {
                    if !self.file_requested {
                        // let result = self.client.borrow_mut().disconnect().unwrap();
                        println!("[{}]: {}", self.name, "Requesting transfer".green());
                        self.client.borrow_mut().request_inbound_transfer(api::InboundTransferRequest {
                            id: "123".to_string(),
                            file_info: Some(api::FileInfo {
                                name: "test.txt".to_string(),
                                size: 3,
                                mode: 0o644,
                            }),
                        }).unwrap();
                        self.file_requested = true;
                        self.process_events();
                    }
                },
                ClientEvent::Disconnected => {
                    println!("[{}]: {}", self.name, "Disconnected by server".green());
                    self.stop();
                },
                ClientEvent::TransferAccepted(id) => {
                    if id == "123" {
                        println!("[{}]: {}", self.name, "Transfer accepted".green());
                        //TOOD
                    }
                }
                // ClientEvent::InboundTransferOffered(request) => {
                    // println!("[{}]: {}: {:?}", self.name, "Inbound transfer offer".green(), request);
                    // self.client.borrow_mut().accept_transfer().unwrap();
                    // // let result = self.client.lock().unwrap().reject_transfer().unwrap();
                    // self.process_events();
                // }
                other => {
                    println!("[{}]: {}: {:?}", self.name, "Unknown result".red(), other);
                    self.stop();
                }
            };
        }
    }

    fn stop(&mut self) {
        self.cancellation_token_source.cancel();
        self.stopped = true;
    }
}

fn main() {
    let old_mode = enable_raw_mode(0);
    ctrlc::set_handler(move || {
        restore_mode(0, old_mode);
        std::process::exit(1);
    }).unwrap();

    let mut args = std::env::args();
    let _ = args.next();
    if let Some(arg) = args.next() {
        App::new(arg).run();
    }

    restore_mode(0, old_mode);
}
