use ctrlc;
use pty::fork::Fork;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Mutex, Arc};
use std::thread;
use cancellation::*;
use colored::*;

use toffee::api::message::Content;
use toffee::{Client, ClientResult};
use toffee::{MessageReader, MessageWriter, MessageOutput, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT};
use toffee::pty::{enable_raw_mode, restore_mode};


struct App<'a> {
    master: pty::fork::Master,
    fork: pty::fork::Fork,
    name: String,
    client: Arc<Mutex<Client<'a>>>,
    stopped: bool,
    old_mode: termios::Termios,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String) -> Self {
        let old_mode = enable_raw_mode(0);
        let fork = Fork::from_ptmx().unwrap();

        ctrlc::set_handler(move || {
            restore_mode(0, old_mode);
            std::process::exit(1);
        })
        .unwrap();

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
            fork,
            name,
            client: Arc::new(Mutex::new(Client::new(
                MessageWriter::new(TransportWriter::new(SERVER_TRANSPORT, Box::new(master.clone())))
            ))),
            stopped: false,
            old_mode,
            cancellation_token_source: CancellationTokenSource::new(),
        };

        return _self
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
                },
                MessageOutput::Message(msg) => {
                    let returned = self.client.lock().unwrap().feed_message(msg);
                    match returned {
                        Ok(result) => {
                            self.process_result(result)
                        },
                        Err(err) => {
                            println!("Server error: {}", err.to_string());
                            self.stop();
                        }
                    }
                },
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
                ()
            },
            ClientResult::InboundTransferOffered(request) => {
                println!("[{}]: {}: {:?}", self.name, "Inbound transfer offer".green(), request);
                let result = self.client.lock().unwrap().accept_transfer().unwrap();
                // let result = self.client.lock().unwrap().reject_transfer().unwrap();
                self.process_result(result);
            }
            other => {
                println!("[{}]: {}: {:?}", self.name, "Unknown result".red(), other);
                self.stop();
            }
        };
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
