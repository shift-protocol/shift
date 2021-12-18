use cancellation::*;
use colored::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Mutex};
use shift::api;
use shift::helpers::send_file;
use shift::{
    Client, ClientEvent, MessageOutput, MessageReader, MessageWriter, TransportWriter, TRANSPORT, OpenFile
};

type ProgressCallback<'a> = Box<dyn FnMut(&OpenFile, u64, u64) + Send + 'a>;

pub struct FileClient<'a> {
    name: String,
    buffer_size: usize,
    client: Arc<Mutex<Client<'a>>>,
    current_path: Option<String>,
    output: Option<Box<dyn Write + Send>>,
    open_file: Option<File>,
    send_progress_callback: Option<ProgressCallback<'a>>,
}

pub trait FileClientDelegate<'a> {
    fn on_idle(&mut self, _client: &mut FileClient<'a>) {}
    fn on_inbound_transfer_request(&mut self, _request: &api::SendRequest) -> bool {
        false
    }
    fn on_inbound_transfer_file(&mut self, _file: &api::OpenFile) -> Option<PathBuf> {
        None
    }
    fn on_outbound_transfer_request(&mut self, _request: &api::ReceiveRequest, _client: &mut FileClient<'a>) {}
    fn on_transfer_closed(&mut self) {}
    fn on_disconnect(&mut self) {}
}

impl<'a> FileClient<'a> {
    pub fn new(
        name: String,
        data_stream_out: Box<dyn Write + Send>,
        output: Option<Box<dyn Write + Send>>,
    ) -> Self {
        Self {
            name,
            buffer_size: 1, // TODO
            client: Arc::new(Mutex::new(Client::new(MessageWriter::new(
                TransportWriter::new(TRANSPORT, Box::new(data_stream_out)),
            )))),
            current_path: None,
            output,
            open_file: None,
            send_progress_callback: None,
        }
    }

    pub fn run<D, S>(
        &mut self,
        announce: bool,
        input: &mut S,
        delegate: &mut D,
        token: &CancellationToken,
    ) where
        D: FileClientDelegate<'a> + Send,
        S: Read + Send,
    {
        if announce {
            self.client.lock().unwrap().start().unwrap();
        }

        let input_closed = Arc::new(AtomicBool::new(false));

        crossbeam::scope(|scope| {
            let (tx, rx) = std::sync::mpsc::channel();

            let reader_thread = scope.spawn({
                let client = self.client.clone();
                let mut output = self.output.take();
                let input_closed = input_closed.clone();
                let tx = tx.clone();
                move |_| {
                    tx.send(0).unwrap();
                    let mut reader = MessageReader::new(TRANSPORT);
                    for msg in reader.feed_from(input, &token) {
                        match msg {
                            MessageOutput::Message(msg) => {
                                client.lock().unwrap().feed_message(msg).unwrap();
                                tx.send(0).unwrap();
                            }
                            MessageOutput::Passthrough(data) => {
                                if let Some(ref mut output) = output {
                                    output.write(&data).unwrap();
                                    output.flush().unwrap();
                                }
                            }
                        }
                    }
                    input_closed.store(true, Ordering::Relaxed);
                    tx.send(0).unwrap();
                }
            });

            println!("[{}]: {}", self.name, "Connected".green());

            'event_loop: loop {
                let events = { self.client.lock().unwrap().take_events() };
                if events.len() == 0 {
                    rx.recv().unwrap();
                }
                {
                    if input_closed.load(Ordering::Relaxed) {
                        break 'event_loop;
                    }
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
                        ClientEvent::InboundTransferOffered(request) => {
                            if delegate.on_inbound_transfer_request(&request) {
                                self.client.lock().unwrap().accept_transfer().unwrap();
                            } else {
                                self.client.lock().unwrap().reject_transfer().unwrap();
                            }
                        }
                        ClientEvent::InboundFileOpening(_, file) => {
                            match delegate.on_inbound_transfer_file(&file) {
                                Some(path) => {
                                    let mut file;
                                    if path.exists() {
                                        file = OpenOptions::new()
                                            .append(true)
                                            .open(path.clone())
                                            .unwrap();
                                    } else {
                                        file = File::create(path.clone()).unwrap();
                                    }
                                    let position = file.seek(SeekFrom::End(0)).unwrap();

                                    self.client
                                        .lock()
                                        .unwrap()
                                        .confirm_file_opened(api::FileOpened {
                                            continue_from: position,
                                        })
                                        .unwrap();

                                    self.open_file = Some(file);
                                }
                                None => {
                                    self.client.lock().unwrap().close_transfer().unwrap();
                                }
                            }
                        }
                        ClientEvent::OutboundTransferOffered(request) => {
                            delegate.on_outbound_transfer_request(&request, self);
                        }
                        ClientEvent::Chunk(chunk) => {
                            if let Some(file) = &mut self.open_file {
                                file.write(&chunk.data).unwrap();
                            }
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
                        ClientEvent::FileTransferStarted(open_file, response) => {
                            println!("[{}]: {}: {:?}", self.name, "File open".green(), response);
                            let path = self.current_path.clone().expect("Current file not set");
                            let mut callback = self.send_progress_callback.take().expect("Callback no set");
                            scope.spawn({
                                let client = self.client.clone();
                                let tx = tx.clone();
                                let buffer_size = self.buffer_size;
                                let open_file = open_file.clone();
                                move |_| {
                                    send_file(client, response.continue_from, path, buffer_size, &mut |sent, total| {
                                        callback(&open_file, sent, total);
                                    }).unwrap();
                                    tx.send(0).unwrap();
                                }
                            });
                        }
                        ClientEvent::FileClosed(_) => {
                            self.open_file = None;
                            self.client.lock().unwrap().close_transfer().unwrap();
                        }
                        ClientEvent::TransferClosed => {
                            println!("[{}]: {}", self.name, "Transfer closed".green());
                            delegate.on_transfer_closed();
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
        self.client
            .lock()
            .unwrap()
            .request_inbound_transfer(api::ReceiveRequest {
                allow_directories: false,
            })
            .unwrap();
    }

    pub fn send_file(&mut self, path: &Path, callback: ProgressCallback<'a>) {
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode();

        self.send_progress_callback = Some(callback);

        println!("[{}]: {}", self.name, "Requesting transfer".green());
        self.client
            .lock()
            .unwrap()
            .request_outbound_transfer(api::SendRequest {
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
