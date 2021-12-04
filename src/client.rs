use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use pty::fork::Fork;
use std::thread;
use termios::{self, Termios};
use std::sync::{Arc, Mutex};
use ctrlc;

mod constants;
mod reader;
use reader::TransportReader;

fn main() {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    let mut tr = TransportReader::new(|data| {
        println!("Packet: {:?}", std::str::from_utf8(data));
    });

    let fork = Fork::from_ptmx().unwrap();

    if fork.is_child().ok().is_some() {
        let mut args = std::env::args();
        args.next();
        println!("{:?}", args);
        Command::new(args.next().expect("No program name"))
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn().unwrap().wait();
        std::process::exit(0);
    }

    let mut termios = Termios::from_fd(0).unwrap();
    let old_termios = termios;
    termios.c_iflag &= !(termios::PARMRK | termios::ISTRIP | termios::INLCR | termios::IGNCR | termios::ICRNL | termios::IXON);
    termios.c_oflag &= !termios::OPOST;
    termios.c_lflag &= !(termios::ECHO | termios::ECHONL | termios::ICANON | termios::IEXTEN);
    termios.c_cflag &= !(termios::CSIZE | termios::PARENB);
    termios.c_cflag |= termios::CS8;
    termios.c_cc[termios::VMIN] = 1;
    termios::tcsetattr(0, termios::TCSANOW, &termios).unwrap();

    ctrlc::set_handler(move || {
        termios::tcsetattr(0, termios::TCSANOW, &old_termios).unwrap();
        std::process::exit(1);
    }).unwrap();

    let mut master_read = fork.is_parent().ok().unwrap();
    let mut master_write = master_read.clone();

    let writer = thread::spawn(move || {
        let mut buf = [0; 1024];
        loop {
            match stdin.read(&mut buf) {
                Ok(size) => {
                    if size == 0 {
                        break
                    }
                    if master_write.write(&buf[..size]).is_err() {
                        break
                    }
                }
                Err(e) => {
                    panic!("read error: {}", e);
                }
            }
        }
    });

    let reader = thread::spawn(move || {
        let mut buf = [0; 1024];
        loop {
            match master_read.read(&mut buf) {
                Ok(size) => {
                    if size == 0 {
                        break
                    }
                    for part in tr.feed(&buf[..size]) {
                        if part.len() > 0{
                            stdout.write(part).unwrap();
                        }
                    }
                    stdout.flush();
                }
                Err(e) => {
                    break
                }
            }
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();

    termios::tcsetattr(0, termios::TCSANOW, &old_termios).unwrap();
}
