use base64;
use std::io::{self, Write};
use twoway;

#[derive(Clone)]
pub struct TransportConfig<'a> {
    pub prefix: &'a [u8],
    pub suffix: &'a [u8],
}

pub struct TransportReader<'a> {
    config: TransportConfig<'a>,
    buffer: Vec<u8>,
    callback: Box<dyn FnMut(&[u8]) -> () + Send + 'a>,
    in_sequence: bool,
}

impl<'a> TransportReader<'a> {
    pub fn new(config: TransportConfig<'a>, callback: impl FnMut(&[u8]) -> () + Send + 'a) -> Self {
        Self {
            buffer: vec![],
            config: config,
            callback: Box::new(callback),
            in_sequence: false,
        }
    }

    pub fn feed<'b>(&mut self, data: &'b [u8]) -> Vec<&'b [u8]> {
        let mut views = Vec::new();
        let mut remaining_data = data;

        if self.in_sequence {
            remaining_data = self.feed_remainder(remaining_data);
        }

        loop {
            match twoway::find_bytes(remaining_data, self.config.prefix) {
                Some(start_index) => {
                    views.push(&remaining_data[..start_index]);
                    remaining_data = &remaining_data[start_index + self.config.prefix.len()..];
                    remaining_data = self.feed_remainder(remaining_data);
                }
                None => {
                    views.push(&remaining_data[..]);
                    break;
                }
            }
        }

        views
    }

    fn feed_remainder<'b>(&mut self, data: &'b [u8]) -> &'b [u8] {
        match twoway::find_bytes(&data, self.config.suffix) {
            Some(length) => {
                let encoded_packet = &data[..length];
                if let Some(packet) = base64::decode(encoded_packet).ok() {
                    (self.callback)(&packet);
                }
                self.in_sequence = false;
                return &data[length + self.config.suffix.len()..];
            }
            None => {
                self.in_sequence = true;
                self.buffer.extend_from_slice(&data);
                return &[];
            }
        }
    }
}

pub struct TransportWriter<'a> {
    config: TransportConfig<'a>,
    stream: Box<dyn Write + Send + 'a>,
}

impl<'a> TransportWriter<'a> {
    pub fn new(config: TransportConfig<'a>, stream: Box<dyn Write + Send>) -> Self {
        Self {
            config,
            stream,
        }
    }

    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(self.config.prefix)?;
        self.stream.write_all(base64::encode(data).as_bytes())?;
        self.stream.write_all(self.config.suffix)?;
        self.stream.flush()?;
        Ok(())
    }
}
