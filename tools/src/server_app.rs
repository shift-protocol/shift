use anyhow::{anyhow, Result};
use cancellation::*;
use clap::{self, Parser};
use colored::*;
use ctrlc;
use path_clean::PathClean;
use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use terminal_size::{terminal_size, Height, Width};

use shift::api;
use shift::pty::{enable_raw_mode, restore_mode};

mod file_client;
use file_client::{FileClient, FileClientDelegate};

#[derive(Parser, Debug)]
#[clap(version)]
struct Cli {
    #[clap(short, long)]
    directory: String,

    #[clap(multiple_values = true)]
    args: Vec<String>,
}

struct App<'a> {
    pty: Option<PtyPair>,
    work_dir: String,
    client: Arc<Mutex<FileClient<'a>>>,
    old_mode: termios::Termios,
    cancellation_token_source: CancellationTokenSource,

    current_inbound_transfer: Option<api::SendRequest>,
}

impl<'a> App<'a> {
    pub fn new(args: Cli) -> Result<Self> {
        let pty_system = native_pty_system();
        let pty_pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            // Not all systems support pixel_width, pixel_height,
            // but it is good practice to set it to something
            // that matches the size of the selected font.  That
            // is more complex than can be shown here in this
            // brief example though!
            pixel_width: 0,
            pixel_height: 0,
        })?;

        resize_pty(&pty_pair)?;

        println!("Starting {}", args.args[0]);
        let mut cmd = CommandBuilder::new(&args.args[0]);
        cmd.cwd(std::env::current_dir()?);
        cmd.args(&args.args[1..]);

        pty_pair
            .slave
            .spawn_command(cmd)
            .expect("Could not spawn command");

        let old_mode = enable_raw_mode(0)?;

        // signal_hook::flag::register(signal_hook::consts::SIGWINCH, Arc::clone(&term))?;

        ctrlc::set_handler(move || {
            restore_mode(0, old_mode).expect("Failed to restore TTY mode");
            std::process::exit(1);
        })?;

        thread::spawn({
            let mut master = pty_pair.master.try_clone_writer().unwrap();
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

        let writer = pty_pair.master.try_clone_writer()?;
        let mut _self = Self {
            pty: Some(pty_pair),
            work_dir: args.directory,
            client: Arc::new(Mutex::new(FileClient::new(
                "server".to_string(),
                Box::new(writer),
                Some(Box::new(io::stdout())),
            ))),
            old_mode,
            cancellation_token_source: CancellationTokenSource::new(),
            current_inbound_transfer: None,
        };

        Ok(_self)
    }

    pub fn run(mut self) -> Result<()> {
        let token = self.cancellation_token_source.token().clone();
        let client = self.client.clone();
        let pty = self.pty.take().expect("PTY not set");
        let mut master = pty.master.try_clone_reader().unwrap();

        thread::spawn(move || {
            for _ in &mut signal_hook::iterator::SignalsInfo::<
                signal_hook::iterator::exfiltrator::SignalOnly,
            >::new(&vec![signal_hook::consts::signal::SIGWINCH])
            .unwrap()
            {
                resize_pty(&pty).expect("Failed to resize PTY");
            }
        });

        client
            .lock()
            .unwrap()
            .run(false, &mut master, &mut self, &token)?;

        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        println!("Stopping");
        self.cancellation_token_source.cancel();
        restore_mode(0, self.old_mode)?;
        Ok(())
    }
}

fn resize_pty(pty: &PtyPair) -> Result<()> {
    if let Some((Width(w), Height(h))) = terminal_size() {
        pty.master.resize(PtySize {
            rows: h,
            cols: w,
            pixel_width: 0,
            pixel_height: 0,
        })?;
    }
    Ok(())
}

impl<'a> FileClientDelegate<'a> for App<'a> {
    fn on_inbound_transfer_request(&mut self, request: &api::SendRequest) -> bool {
        self.current_inbound_transfer = Some(request.clone());
        return true;
    }

    fn on_inbound_transfer_file(&mut self, file: &api::OpenFile) -> Result<Option<PathBuf>> {
        let transfer = self.current_inbound_transfer.clone();
        let rel_path = Path::new(
            &transfer
                .ok_or(anyhow!("No active transfer"))?
                .file_info
                .ok_or(anyhow!("Missing file info"))?
                .name,
        )
        .join(
            file.file_info
                .clone()
                .ok_or(anyhow!("Missing file info in request"))?
                .name,
        )
        .clean();
        let path = PathBuf::from(&self.work_dir).join(rel_path);
        std::fs::create_dir_all(path.parent().expect("Cannot handle root directory"))?;
        return Ok(Some(path));
    }

    fn on_outbound_transfer_request(
        &mut self,
        _request: &api::ReceiveRequest,
        client: &mut FileClient<'a>,
    ) -> Result<()> {
        let path = Path::new(&self.work_dir);
        let item = path.read_dir()?.next();
        match item {
            Some(item) => {
                client.send(&item?.path(), Box::new(|_, _, _| {}))?;
            }
            None => {
                println!("[server]: {}", "No files to send".green());
                client.disconnect()?;
            }
        }
        Ok(())
    }

    fn on_transfer_closed(&mut self) {
        self.current_inbound_transfer = None;
    }

    fn on_disconnect(&mut self) -> Result<()> {
        self.stop()
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    App::new(cli)?.run()?;
    Ok(())
}
