use std::io;
use toffee::api::message::Content;
use toffee::MessageWriter;
use toffee::TransportWriter;

fn main() {
    let mut stream = io::stdout();
    let mut tl = MessageWriter::new(TransportWriter::new(&mut stream));

    let x = Content::ClientInit(toffee::api::ClientInit {
        version: 1,
        features: vec![],
    });
    tl.write(x).unwrap();
}
