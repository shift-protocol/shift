use anyhow::{anyhow, Result};
use cancellation::*;
use shift::api;
use shift::helpers::send_file;
use shift::{
    MessageOutput, MessageReader, MessageWriter, OpenFile, ShiftClient, ShiftClientEvent,
    TransportWriter, TRANSPORT,
};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, atomic::Ordering, Arc, Mutex};

#[cfg(target_family = "unix")]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_family = "unix")]
fn get_file_mode(metadata: &Metadata) -> u32 {
    metadata.permissions().mode()
}

#[cfg(target_family = "windows")]
fn get_file_mode(metadata: &Metadata) -> u32 {
    if metadata.is_dir() {
        0o755
    } else {
        0o644
    }
}

type ProgressCallback<'a> = Box<dyn FnMut(&OpenFile, u64, u64) + Send + 'a>;

pub struct ShiftFileClient<'a> {
    buffer_size: usize,
    client: Arc<Mutex<ShiftClient<'a>>>,
    current_transfer_path: Option<PathBuf>,
    current_file_path: Option<PathBuf>,
    remaining_files_to_send: Option<Vec<PathBuf>>,
    total_bytes_sent: u64,
    total_bytes_to_send: u64,
    output: Option<Box<dyn Write + Send>>,
    open_file: Option<File>,
    send_progress_callback: Option<Arc<Mutex<ProgressCallback<'a>>>>,
}

pub trait ShiftFileClientDelegate<'a> {
    fn on_idle(&mut self, _client: &mut ShiftFileClient<'a>) -> Result<()> {
        Ok(())
    }
    fn on_inbound_transfer_request(&mut self, _request: &api::SendRequest) -> bool {
        false
    }
    fn on_inbound_transfer_file(&mut self, _file: &api::OpenFile) -> Result<Option<PathBuf>> {
        Ok(None)
    }
    fn on_outbound_transfer_request(
        &mut self,
        _request: &api::ReceiveRequest,
        _client: &mut ShiftFileClient<'a>,
    ) -> Result<()> {
        Ok(())
    }
    fn on_tick(&mut self) {}
    fn on_transfer_closed(&mut self) {}
    fn on_disconnect(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<'a> ShiftFileClient<'a> {
    pub fn new(
        data_stream_out: Box<dyn Write + Send>,
        output: Option<Box<dyn Write + Send>>,
    ) -> Self {
        Self {
            buffer_size: 1024 * 512,
            client: Arc::new(Mutex::new(ShiftClient::new(MessageWriter::new(
                TransportWriter::new(TRANSPORT, Box::new(data_stream_out)),
            )))),
            current_transfer_path: None,
            current_file_path: None,
            remaining_files_to_send: None,
            total_bytes_sent: 0,
            total_bytes_to_send: 0,
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
    ) -> Result<()>
    where
        D: ShiftFileClientDelegate<'a> + Send,
        S: Read + Send,
    {
        if announce {
            self.client.lock().unwrap().start()?;
        }

        let stop = Arc::new(AtomicBool::new(false));

        let loop_result = crossbeam::scope(|scope| -> Result<()> {
            let (tx, rx) = std::sync::mpsc::channel();

            let reader_thread = scope.spawn({
                let client = self.client.clone();
                let mut output = self.output.take();
                let stop = stop.clone();
                let tx = tx.clone();
                move |_| -> Result<()> {
                    tx.send(0)?;
                    let mut reader = MessageReader::new(TRANSPORT);
                    for msg in reader.feed_from(input, token) {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        match msg {
                            MessageOutput::Message(msg) => {
                                client.lock().unwrap().feed_message(msg)?;
                                tx.send(0)?;
                            }
                            MessageOutput::Passthrough(data) => {
                                if let Some(ref mut output) = output {
                                    output.write_all(&data)?;
                                    output.flush()?;
                                }
                            }
                        }
                    }
                    stop.store(true, Ordering::Relaxed);
                    tx.send(0)?;
                    Ok(())
                }
            });

            (|| {
                'event_loop: loop {
                    let events = { self.client.lock().unwrap().take_events() };
                    if events.is_empty() {
                        rx.recv()?;
                    }
                    {
                        if stop.load(Ordering::Relaxed) {
                            break 'event_loop;
                        }
                    }
                    delegate.on_tick();
                    for event in events {
                        match event {
                            ShiftClientEvent::Connected => {
                                delegate.on_idle(self)?;
                            }
                            ShiftClientEvent::Disconnected => {
                                break 'event_loop;
                            }
                            ShiftClientEvent::InboundTransferOffered(request) => {
                                if delegate.on_inbound_transfer_request(&request) {
                                    self.client.lock().unwrap().accept_transfer()?;
                                } else {
                                    self.client.lock().unwrap().reject_transfer()?;
                                }
                            }
                            ShiftClientEvent::InboundFileOpening(_, file) => {
                                match delegate.on_inbound_transfer_file(&file) {
                                    Ok(Some(path)) => {
                                        std::fs::create_dir_all(path.parent().ok_or(anyhow!(
                                            "Cannot operate on filesystem root"
                                        ))?)?;
                                        if file
                                            .file_info
                                            .ok_or(anyhow!("Missing file info in request"))?
                                            .mode
                                            & 0o40000
                                            != 0
                                        {
                                            self.client.lock().unwrap().confirm_file_opened(
                                                api::FileOpened { continue_from: 0 },
                                            )?;
                                        } else {
                                            let mut file;
                                            if path.exists() {
                                                file = OpenOptions::new()
                                                    .append(true)
                                                    .open(path.clone())?;
                                            } else {
                                                file = File::create(path.clone())?;
                                            }
                                            let position = file.seek(SeekFrom::End(0))?;

                                            self.client.lock().unwrap().confirm_file_opened(
                                                api::FileOpened {
                                                    continue_from: position,
                                                },
                                            )?;

                                            self.open_file = Some(file);
                                        }
                                    }
                                    Ok(None) | Err(_) => {
                                        self.client.lock().unwrap().close_transfer()?;
                                    }
                                }
                            }
                            ShiftClientEvent::OutboundTransferOffered(request) => {
                                delegate.on_outbound_transfer_request(&request, self)?;
                            }
                            ShiftClientEvent::Chunk(chunk) => {
                                println!(
                                    "chynk len {:?} for {:?}",
                                    chunk.data.len(),
                                    self.open_file.is_some()
                                );
                                if let Some(file) = &mut self.open_file {
                                    let position = file.seek(SeekFrom::Current(0))?;
                                    if position != chunk.offset {
                                        return Err(anyhow!(
                                            "Chunk offset does not match file position"
                                        ));
                                    }
                                    file.write_all(&chunk.data)?;
                                    self.client.lock().unwrap().acknolwedge_chunk()?;
                                    println!("now at {:?}", position);
                                }
                            }
                            ShiftClientEvent::TransferAccepted() => {
                                self.maybe_send_next_file()?;
                            }
                            ShiftClientEvent::FileTransferStarted(open_file, response) => {
                                let full_path = std::fs::canonicalize(
                                    self.current_transfer_path
                                        .clone()
                                        .ok_or(anyhow!("Missing transfer path"))?
                                        .join(
                                            self.current_file_path
                                                .clone()
                                                .ok_or(anyhow!("No current file"))?,
                                        ),
                                )?;

                                if full_path.is_dir() {
                                    self.client.lock().unwrap().close_file()?;
                                    continue;
                                }

                                let callback = self
                                    .send_progress_callback
                                    .clone()
                                    .ok_or(anyhow!("Missing callback"))?;
                                scope.spawn({
                                    let client = self.client.clone();
                                    let tx = tx.clone();
                                    let buffer_size = self.buffer_size;
                                    let open_file = open_file.clone();
                                    let total_bytes_sent = self.total_bytes_sent;
                                    let total_bytes_to_send = self.total_bytes_to_send;
                                    move |_| -> Result<()> {
                                        send_file(
                                            client,
                                            response.continue_from,
                                            &full_path,
                                            buffer_size,
                                            &mut |sent, _| {
                                                callback.lock().unwrap()(
                                                    &open_file,
                                                    total_bytes_sent + sent,
                                                    total_bytes_to_send,
                                                );
                                            },
                                        )?;
                                        tx.send(0)?;
                                        Ok(())
                                    }
                                });
                            }
                            ShiftClientEvent::FileClosed(f) => {
                                if let Some(file) = &mut self.open_file {
                                    // file.flush()?;
                                    // file.sync_all()?;
                                    println!("now at {:?}", file.seek(SeekFrom::Current(0)));
                                }
                                self.open_file = None;
                                self.total_bytes_sent += f.info.size;
                                self.maybe_send_next_file()?;
                            }
                            ShiftClientEvent::TransferClosed => {
                                self.remaining_files_to_send = None;
                                delegate.on_transfer_closed();
                                delegate.on_idle(self)?;
                            }
                            other => {
                                return Err(anyhow!("Unknown event: {:?}", other));
                            }
                        };
                        delegate.on_tick();
                    }
                }

                Ok(())
            })()
            .map_err(|e| {
                stop.store(true, Ordering::Relaxed);
                e
            })?;

            reader_thread.join().expect("Failure in reader thread")?;
            Ok(())
        });

        loop_result.map_err(|_| anyhow!("Panic in a service thread"))??;

        Ok(())
    }

    fn maybe_send_next_file(&mut self) -> Result<()> {
        if let Some(remaining_files_to_send) = &mut self.remaining_files_to_send {
            self.current_file_path = remaining_files_to_send.pop();

            match &self.current_file_path {
                Some(path) => {
                    let full_path = std::fs::canonicalize(
                        self.current_transfer_path
                            .clone()
                            .ok_or(anyhow!("Missing transfer path"))?
                            .join(path.clone()),
                    )?;
                    let meta = std::fs::metadata(&full_path)?;
                    self.client.lock().unwrap().open_file(api::OpenFile {
                        file_info: Some(api::FileInfo {
                            name: path.to_string_lossy().to_string(),
                            size: meta.len(),
                            mode: get_file_mode(&meta),
                        }),
                    })?;
                }
                None => {
                    self.client.lock().unwrap().close_transfer()?;
                }
            }
        }

        Ok(())
    }

    pub fn receive(&mut self) -> Result<()> {
        self.client
            .lock()
            .unwrap()
            .request_inbound_transfer(api::ReceiveRequest {
                allow_directories: false,
                allow_multiple: true,
            })
    }

    fn send_file(&mut self, path: &Path, callback: ProgressCallback<'a>) -> Result<()> {
        let meta = std::fs::metadata(&path)?;
        let mode = get_file_mode(&meta);

        self.send_progress_callback = Some(Arc::new(Mutex::new(callback)));

        self.client
            .lock()
            .unwrap()
            .request_outbound_transfer(api::SendRequest {
                file_info: Some(api::FileInfo {
                    name: path
                        .file_name()
                        .and_then(|x| x.to_str())
                        .map(|x| x.to_string())
                        .ok_or(anyhow!("Could not determine file name"))?,
                    size: meta.len(),
                    mode,
                }),
            })?;

        self.current_transfer_path = Some(PathBuf::from(path));
        self.total_bytes_sent = 0;
        self.total_bytes_to_send = meta.len();
        Ok(())
    }

    pub fn send(&mut self, path: &Path, callback: ProgressCallback<'a>) -> Result<()> {
        self.send_file(path, callback)?;

        if path.is_dir() {
            self.current_transfer_path = Some(PathBuf::from(path));

            let entries = walkdir::WalkDir::new(path).into_iter().try_fold(
                vec![],
                |mut acc, entry| -> Result<_> {
                    acc.push(entry?);
                    Ok(acc)
                },
            )?;

            self.remaining_files_to_send = Some(
                entries
                    .iter()
                    .try_fold(vec![], |mut acc, entry| -> Result<_> {
                        acc.push(
                            pathdiff::diff_paths(entry.path(), path)
                                .ok_or(anyhow!("Could not determine relative path"))?,
                        );
                        Ok(acc)
                    })?
                    .into_iter()
                    .filter(|p| !p.eq(&PathBuf::from("")))
                    .collect(),
            );

            self.total_bytes_sent = 0;
            self.total_bytes_to_send = entries
                .iter()
                .try_fold(0, |acc, p| -> Result<u64> { Ok(acc + p.metadata()?.len()) })?;
        } else {
            self.remaining_files_to_send = Some(vec![PathBuf::from(".")]);
        }

        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<()> {
        self.client.lock().unwrap().disconnect()
    }
}
