use bytes::{Bytes, BytesMut};
use cancellation::*;
use std::io::{self, Read, Write};

#[derive(Clone)]
pub struct TransportConfig<'a> {
    pub prefix: &'a [u8],
    pub suffix: &'a [u8],
}

pub struct TransportReader<'a> {
    config: TransportConfig<'a>,
    buffer: BytesMut,
}

#[derive(Debug, PartialEq)]
pub enum TransportOutput {
    Passthrough(Bytes),
    Packet(Bytes),
}

pub struct TransportFeeder<'a> {
    reader: &'a mut TransportReader<'a>,
    stream: &'a mut dyn Read,
    data_buffer: Vec<u8>,
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
            data_buffer: vec![0; 1024 * 512],
            result_buffer: Vec::new(),
            ct,
        }
    }
}

impl<'a> Iterator for TransportFeeder<'a> {
    type Item = TransportOutput;

    fn next(&mut self) -> Option<TransportOutput> {
        if self.ct.result().is_err() {
            return None;
        }
        while self.result_buffer.is_empty() {
            let size = self.stream.read(&mut self.data_buffer).expect("read error");
            if size == 0 {
                return None;
            }
            self.result_buffer
                .append(&mut self.reader.feed(&self.data_buffer[..size]));
        }

        if self.result_buffer.is_empty() {
            return None;
        }

        Some(self.result_buffer.remove(0))
    }
}

impl<'a> TransportReader<'a> {
    pub fn new(config: TransportConfig<'a>) -> Self {
        Self {
            buffer: BytesMut::new(),
            config,
        }
    }

    pub fn feed_from(
        &'a mut self,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> TransportFeeder<'a> {
        TransportFeeder::new(self, stream, ct)
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<TransportOutput> {
        self.buffer.extend_from_slice(data);
        let mut result = vec![];

        'outer: loop {
            if self.buffer.is_empty() {
                break;
            }

            match twoway::find_bytes(&self.buffer, self.config.prefix) {
                Some(0) => match twoway::find_bytes(&self.buffer, self.config.suffix) {
                    Some(end_index) => {
                        let content = self
                            .buffer
                            .split_to(end_index + self.config.suffix.len())
                            .split_to(end_index)
                            .split_off(self.config.prefix.len());
                        if let Ok(content) = base64::decode(content) {
                            result.push(TransportOutput::Packet(Bytes::from(content)));
                        }
                        continue;
                    }
                    None => {
                        break;
                    }
                },
                Some(start_index) => {
                    let passthrough = self.buffer.split_to(start_index);
                    result.push(TransportOutput::Passthrough(passthrough.freeze()));
                    continue;
                }
                None => {
                    for prefix_len in
                        1..=std::cmp::min(self.buffer.len(), self.config.prefix.len() - 1)
                    {
                        if self.buffer[self.buffer.len() - prefix_len..]
                            == self.config.prefix[..prefix_len]
                        {
                            // Could potentially be a prefix of the transport prefix
                            break 'outer;
                        }
                    }
                    let mut buf = BytesMut::new();
                    std::mem::swap(&mut self.buffer, &mut buf);
                    result.push(TransportOutput::Passthrough(buf.freeze()));
                    break;
                }
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
