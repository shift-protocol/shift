use base64;
use bytes::{Bytes, BytesMut};
use cancellation::*;
use std::io::{self, Read, Write};
use twoway;

#[derive(Clone)]
pub struct TransportConfig<'a> {
    pub prefix: &'a [u8],
    pub suffix: &'a [u8],
}

pub struct TransportReader<'a> {
    config: TransportConfig<'a>,
    buffer: Vec<u8>,
    in_sequence: bool,
}

#[derive(Debug)]
pub enum TransportOutput {
    Passthrough(Bytes),
    Packet(Bytes),
}

pub struct TransportFeeder<'a> {
    reader: &'a mut TransportReader<'a>,
    stream: &'a mut dyn Read,
    data_buffer: [u8; 1024 * 512],
    result_buffer: Vec<TransportOutput>,
    ct: &'a CancellationToken,
}

impl<'a> TransportFeeder<'a> {
    pub fn new(
        reader: &'a mut TransportReader<'a>,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> Self {
        Self {
            reader,
            stream,
            data_buffer: [0; 1024 * 512],
            result_buffer: Vec::new(),
            ct,
        }
    }
}

impl<'a> Iterator for TransportFeeder<'a> {
    type Item = TransportOutput;

    fn next(&mut self) -> Option<TransportOutput> {
        if let None = self.ct.result().ok() {
            return None;
        }
        while self.result_buffer.len() == 0 {
            let size = self.stream.read(&mut self.data_buffer).expect("read error");
            if size == 0 {
                return None;
            }
            self.result_buffer
                .append(&mut self.reader.feed(&self.data_buffer[..size]));
        }

        if self.result_buffer.len() == 0 {
            return None;
        }

        Some(self.result_buffer.remove(0))
    }
}

impl<'a> TransportReader<'a> {
    pub fn new(config: TransportConfig<'a>) -> Self {
        Self {
            buffer: vec![],
            config,
            in_sequence: false,
        }
    }

    pub fn feed_from(
        &'a mut self,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> TransportFeeder<'a> {
        return TransportFeeder::new(self, stream, ct);
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<TransportOutput> {
        let mut remaining_data = Bytes::from(BytesMut::from(data));
        let mut result = vec![];
        if self.in_sequence {
            for part in self.feed_remainder(&remaining_data) {
                match part {
                    TransportOutput::Packet(_) => {
                        result.push(part);
                    }
                    TransportOutput::Passthrough(data) => {
                        remaining_data = data;
                    }
                }
            }
        }

        loop {
            match twoway::find_bytes(&remaining_data, self.config.prefix) {
                Some(start_index) => {
                    result.push(TransportOutput::Passthrough(
                        remaining_data.slice(..start_index).clone(),
                    ));
                    remaining_data = remaining_data.slice(start_index + self.config.prefix.len()..);

                    for part in self.feed_remainder(&remaining_data) {
                        match part {
                            TransportOutput::Packet(_) => {
                                result.push(part);
                            }
                            TransportOutput::Passthrough(data) => {
                                remaining_data = data;
                            }
                        }
                    }
                }
                None => {
                    result.push(TransportOutput::Passthrough(remaining_data));
                    break;
                }
            }
        }

        result
    }

    fn feed_remainder(&mut self, data: &Bytes) -> Vec<TransportOutput> {
        let mut result: Vec<TransportOutput> = vec![];
        match twoway::find_bytes(data, self.config.suffix) {
            Some(length) => {
                self.buffer.extend_from_slice(&data[..length]);
                if let Some(packet) = base64::decode(&self.buffer).ok() {
                    result.push(TransportOutput::Packet(Bytes::from(packet)));
                }
                self.buffer = vec![];
                self.in_sequence = false;
                result.push(TransportOutput::Passthrough(
                    data.slice(length + self.config.suffix.len()..),
                ));
            }
            None => {
                self.in_sequence = true;
                self.buffer.extend_from_slice(&data);
            }
        }
        result
    }
}

pub struct TransportWriter<'a> {
    config: TransportConfig<'a>,
    stream: Box<dyn Write + Send + 'a>,
}

impl<'a> TransportWriter<'a> {
    pub fn new(config: TransportConfig<'a>, stream: Box<dyn Write + Send>) -> Self {
        Self { config, stream }
    }

    pub fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(self.config.prefix)?;
        self.stream.write_all(base64::encode(data).as_bytes())?;
        self.stream.write_all(self.config.suffix)?;
        self.stream.flush()?;
        Ok(())
    }
}
