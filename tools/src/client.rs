use std::io;
use toffee::{MessageReader, MessageWriter, MessageOutput, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT, Client, ClientResult};
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
        println!("[{}]: {}", self.name, "Connected".green());
        let token = self.cancellation_token_source.token().clone();
        let _ = self.feed_input(&token);
    }

    fn feed_input(&mut self, ct: &CancellationToken) -> Result<(), OperationCanceled> {
        let mut stdin = io::stdin();

        for msg in MessageReader::new(SERVER_TRANSPORT).feed_from(&mut stdin, ct) {
            if let MessageOutput::Message(msg) = msg {
                let result = self.client.borrow_mut().feed_message(msg).unwrap();
                self.process_result(result);
            }
        }

        Ok(())
    }

    fn process_result(&mut self, res: ClientResult) {
        match res {
            ClientResult::Disconnected => {
                println!("[{}]: {}", self.name, "Disconnected by server".green());
                self.stop();
            },
            ClientResult::Busy => (),
            ClientResult::Idle => {
                if !self.file_requested {
                    // let result = self.client.borrow_mut().disconnect().unwrap();
                    println!("[{}]: {}", self.name, "Requesting file".green());
                    let result = self.client.borrow_mut().request_inbound_transfer(api::InboundTransferRequest {
                        id: "123".to_string(),
                        file_info: Some(api::FileInfo {
                            name: "test.txt".to_string(),
                            size: 3,
                            mode: 0o644,
                        }),
                    }).unwrap();
                    self.file_requested = true;
                    self.process_result(result);
                }
            },
            other => {
                panic!("Unexpected result: {:?}", other);
            }
        };
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
