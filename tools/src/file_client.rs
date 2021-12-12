use cancellation::*;
use colored::*;
use ctrlc;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{mpsc::Receiver, Arc, Mutex};
use toffee::api;
use toffee::helpers::send_file;
use toffee::pty::{enable_raw_mode, restore_mode};
use toffee::{
    Client, ClientEvent, MessageOutput, MessageReader, MessageWriter, TransportWriter,
    CLIENT_TRANSPORT, SERVER_TRANSPORT,
};

pub struct FileClient<'a> {
    name: String,
    client: Arc<Mutex<Client<'a>>>,
    current_path: Option<String>,
}

pub trait FileClientDelegate<'a> {
    fn on_idle(&mut self, client: &mut FileClient<'a>);
}

impl<'a> FileClient<'a> {
    pub fn new(name: String) -> Self {
        let stdout = io::stdout();

        Self {
            name,
            client: Arc::new(Mutex::new(Client::new(MessageWriter::new(
                TransportWriter::new(CLIENT_TRANSPORT, Box::new(stdout)),
            )))),
            current_path: None,
        }
    }

    pub fn run<D>(&mut self, announce: bool, delegate: &mut D, token: &CancellationToken)
    where
        D: FileClientDelegate<'a>,
    {
        if announce {
            self.client.lock().unwrap().start().unwrap();
        }
        crossbeam::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();

            let reader_thread = scope.spawn({
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

            println!("[{}]: {}", self.name, "Connected".green());

            'event_loop: loop {
                let events = { self.client.lock().unwrap().take_events() };
                if events.len() == 0 {
                    rx.recv().unwrap();
                }
                for event in events {
                    match event {
                        ClientEvent::Connected => {
                            delegate.on_idle(self);
                        }
                        ClientEvent::Disconnected => {
                            println!("[{}]: {}", self.name, "Disconnected by server".green());
                            break 'event_loop;
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
                            delegate.on_idle(self);
                        }
                        other => {
                            println!("[{}]: {}: {:?}", self.name, "Unknown event".red(), other);
                            break 'event_loop;
                        }
                    };
                }
            }

            reader_thread.join().unwrap();
        })
        .unwrap();
    }

    pub fn receive(&mut self) {
        self.client.lock().unwrap().request_inbound_transfer(api::ReceiveRequest {
            allow_directories: false,
            allow_multiple: false,
        })
        .unwrap();
    }

    pub fn send_file(&mut self, path: &Path) {
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode();
        println!("[{}]: {}", self.name, "Requesting transfer".green());
        self.client.lock().unwrap().request_outbound_transfer(api::SendRequest {
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

        self.current_path = Some(path.to_string_lossy().to_string());
    }

    pub fn disconnect(&mut self) {
        self.client.lock().unwrap().disconnect().unwrap();
    }
}
