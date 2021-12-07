use ctrlc;
use pty::fork::Fork;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::cell::RefCell;
use std::sync::{Mutex, Arc};
use std::thread;
use cancellation::*;
use colored::*;

use toffee::api::message::Content;
use toffee::{Client, ClientResult};
use toffee::{MessageReader, MessageWriter, MessageOutput, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT};
use toffee::pty::{enable_raw_mode, restore_mode};


struct App<'a> {
    input: Box<dyn Read + Send>,
    name: String,
    client: Arc<Mutex<Client<'a>>>,
    stopped: bool,
    cancellation_token_source: CancellationTokenSource,
}

impl<'a> App<'a> {
    pub fn new(name: String, input: Box<dyn Read + Send>, output: Box<dyn Write + Send>) -> Self {
        let stdout = io::stdout();

        Self {
            input,
            name,
            client: Arc::new(Mutex::new(Client::new(
                MessageWriter::new(TransportWriter::new(CLIENT_TRANSPORT, Box::new(output)))
            ))),
            stopped: false,
            cancellation_token_source: CancellationTokenSource::new(),
        }
    }

    pub fn run(&mut self) {
        let token = self.cancellation_token_source.token().clone();
        self.feed_input(&token);
    }

    fn feed_input(&mut self, ct: &CancellationToken) -> Result<(), OperationCanceled> {
        let client = self.client.clone();
        // self.reader.feed_from(&mut self.input, ct)?;
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
        };
    }

    fn stop(&mut self) {
        self.cancellation_token_source.cancel();
        self.stopped = true;
    }
}

fn _main() {
    // let mut reader = MessageReader::new(SERVER_TRANSPORT, |msg| {
    //     let result = self.client.lock().unwrap().feed_message(msg).unwrap();
    //     self.process_result(result);
    // });

}

fn main() {
    let mut stdin = io::stdin();
    let old_mode = enable_raw_mode(0);
    let fork = Fork::from_ptmx().unwrap();

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

    ctrlc::set_handler(move || {
        restore_mode(0, old_mode);
        std::process::exit(1);
    })
    .unwrap();

    let master = fork.is_parent().ok().unwrap();

    thread::spawn({
        let mut master = master.clone();
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

    let mut master = master.clone();
    let mut message_writer =
        MessageWriter::new(TransportWriter::new(SERVER_TRANSPORT, Box::new(master)));

    let client = Client::new(
        MessageWriter::new(TransportWriter::new(CLIENT_TRANSPORT, Box::new(master)))
    );

    let cancellation_token_source = CancellationTokenSource::new();

    let mut message_reader = MessageReader::new(CLIENT_TRANSPORT);

    let mut stdout = io::stdout();

    for msg in message_reader.feed_from(&mut master, &cancellation_token_source.token().clone()) {
        println!("{:?}", msg);
        match msg {
            MessageOutput::Passthrough(data) => {
                stdout.write(&data).unwrap();
            },
            MessageOutput::Message(data) => {
                match data {
                    Content::Init(init) => {
                        println!("Server: got client init ver: {:?}", init.version);
                        message_writer
                            .write(Content::Init(toffee::api::Init {
                                version: 1,
                                features: vec![],
                            }))
                            .unwrap();
                    },
                    Content::Disconnect(_) => {
                        println!("Server: got disconnect");
                    },
                    msg => {
                        println!("Server: got unknown message: {:?}", msg);
                        message_writer
                            .write(Content::Disconnect(toffee::api::Disconnect { }))
                            .unwrap();
                    },
                }
            }
        }
    }

    restore_mode(0, old_mode);
}
