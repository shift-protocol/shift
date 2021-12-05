use std::io::{self, Read};
use toffee::api::message::Content;
use toffee::{MessageReader, MessageWriter, TransportWriter, CLIENT_TRANSPORT, SERVER_TRANSPORT};

fn main() {
    let mut stdin = io::stdin();
    let stdout = io::stdout();

    let mut writer = MessageWriter::new(TransportWriter::new(CLIENT_TRANSPORT, Box::new(stdout)));
    let mut reader = MessageReader::new(SERVER_TRANSPORT, |data| {
        println!("Client: packet: {:?}", data);
    });

    let x = Content::ClientInit(toffee::api::ClientInit {
        version: 1,
        features: vec![],
    });
    writer.write(x).unwrap();

    let mut buf = [0; 1024];
    loop {
        let size = stdin.read(&mut buf).expect("read error");
        if size == 0 {
            break;
        }
        reader.feed(&buf[0..size]);
    }
}
