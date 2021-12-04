use base64;
use std::io::{self, Write};
use twoway;

use super::constants::*;

pub struct TransportReader<'a> {
    buffer: Vec<u8>,
    callback: Box<dyn Fn(&[u8]) -> () + Send + 'a>,
    in_sequence: bool,
}

impl<'a> TransportReader<'a> {
    pub fn new(callback: impl Fn(&[u8]) -> () + Send + 'a) -> Self {
        Self {
            buffer: vec![],
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
            match twoway::find_bytes(remaining_data, PREFIX) {
                Some(start_index) => {
                    views.push(&remaining_data[..start_index]);
                    remaining_data = &remaining_data[start_index + PREFIX.len()..];
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
        match twoway::find_bytes(&data, SUFFIX) {
            Some(length) => {
                let encoded_packet = &data[..length];
                if let Some(packet) = base64::decode(encoded_packet).ok() {
                    (self.callback)(&packet);
                }
                self.in_sequence = false;
                return &data[length + SUFFIX.len()..];
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
    stream: &'a mut dyn Write,
}

impl<'a> TransportWriter<'a> {
    pub fn new(stream: &'a mut dyn Write) -> Self {
        Self { stream }
    }

    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(PREFIX)?;
        self.stream.write_all(base64::encode(data).as_bytes())?;
        self.stream.write_all(SUFFIX)?;
        Ok(())
    }
}
