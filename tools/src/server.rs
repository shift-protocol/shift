use ctrlc;
use pty::fork::Fork;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use termios::{self, Termios};

use toffee::api::message::Content;
use toffee::{MessageReader, MessageWriter, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT};

fn main() {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

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

    let mut termios = Termios::from_fd(0).unwrap();
    let old_termios = termios;
    termios.c_iflag &= !(termios::PARMRK
        | termios::ISTRIP
        | termios::INLCR
        | termios::IGNCR
        | termios::ICRNL
        | termios::IXON);
    termios.c_oflag &= !termios::OPOST;
    termios.c_lflag &= !(termios::ECHO | termios::ECHONL | termios::ICANON | termios::IEXTEN);
    termios.c_cflag &= !(termios::CSIZE | termios::PARENB);
    termios.c_cflag |= termios::CS8;
    termios.c_cc[termios::VMIN] = 1;
    termios::tcsetattr(0, termios::TCSANOW, &termios).unwrap();

    ctrlc::set_handler(move || {
        termios::tcsetattr(0, termios::TCSANOW, &old_termios).unwrap();
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

    let reader = thread::spawn({
        let mut master = master.clone();
        let mut message_writer =
            MessageWriter::new(TransportWriter::new(SERVER_TRANSPORT, Box::new(master)));

        let mut message_reader = MessageReader::new(CLIENT_TRANSPORT, {
            move |data| {
                println!("Packet: {:?}", data);
                if let Content::ClientInit(init) = data {
                    println!("Server: got client init ver: {:?}", init.version);
                    message_writer
                        .write(Content::ServerInit(toffee::api::ServerInit {
                            version: 1,
                            features: vec![],
                        }))
                        .unwrap();
                }
            }
        });

        move || {
            let mut buf = [0; 1024];
            loop {
                match master.read(&mut buf) {
                    Ok(size) => {
                        if size == 0 {
                            break;
                        }
                        for part in message_reader.feed(&buf[..size]) {
                            if part.len() > 0 {
                                stdout.write(part).unwrap();
                            }
                        }
                        stdout.flush().unwrap();
                    }
                    Err(_) => break,
                }
            }
        }
    });

    reader.join().unwrap();
    termios::tcsetattr(0, termios::TCSANOW, &old_termios).unwrap();
}
