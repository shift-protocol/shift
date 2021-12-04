use std::io::{self, Write};

mod constants;
use constants::*;

pub struct TransportWriter<'a> {
    stream: &'a mut dyn Write
}

impl<'a> TransportWriter<'a> {
    pub fn new(stream: &'a mut dyn Write) -> Self {
        Self { stream }
    }

    pub fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(PREFIX)?;
        self.stream.write_all(data)?;
        self.stream.write_all(SUFFIX)?;
        Ok(())
    }
}

fn main() {
    let mut stream = io::stdout();
    let mut tl = TransportWriter::new(&mut stream);
    tl.send("Hello, world!".as_bytes()).unwrap();
}
