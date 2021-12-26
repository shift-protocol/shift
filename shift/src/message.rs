use bytes::Bytes;
use cancellation::*;
use prost::Message;
use std::io::{self, Read};

use super::api::{self, message::Content};
use super::transport::{
    TransportConfig, TransportFeeder, TransportOutput, TransportReader, TransportWriter,
};

fn decode(output: TransportOutput) -> Option<MessageOutput> {
    match output {
        TransportOutput::Passthrough(data) => return Some(MessageOutput::Passthrough(data)),
        TransportOutput::Packet(data) => {
            if let Ok(msg) = api::Message::decode(data) {
                if let Some(content) = msg.content {
                    return Some(MessageOutput::Message(content));
                }
            }
        }
    }
    None
}

fn decode_from(output: Vec<TransportOutput>) -> Vec<MessageOutput> {
    output.into_iter().filter_map(decode).collect()
}

#[derive(Debug)]
pub enum MessageOutput {
    Passthrough(Bytes),
    Message(Content),
}

pub struct MessageReader<'a> {
    reader: TransportReader<'a>,
}

pub struct MessageFeeder<'a> {
    feeder: TransportFeeder<'a>,
}

impl<'a> MessageFeeder<'a> {
    pub fn new(
        reader: &'a mut TransportReader<'a>,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> Self {
        Self {
            feeder: reader.feed_from(stream, ct),
        }
    }
}

impl<'a> Iterator for MessageFeeder<'a> {
    type Item = MessageOutput;

    fn next(&mut self) -> Option<MessageOutput> {
        self.feeder.next().and_then(decode)
    }
}

impl<'a> MessageReader<'a> {
    pub fn new(config: TransportConfig<'a>) -> Self {
        Self {
            reader: TransportReader::new(config),
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<MessageOutput> {
        let output = self.reader.feed(data);
        decode_from(output)
    }

    pub fn feed_from(
        &'a mut self,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> MessageFeeder<'a> {
        MessageFeeder::new(&mut self.reader, stream, ct)
    }
}

pub struct MessageWriter<'a> {
    writer: TransportWriter<'a>,
}

impl<'a> MessageWriter<'a> {
    pub fn new(writer: TransportWriter<'a>) -> Self {
        Self { writer }
    }

    pub fn write(&mut self, msg: Content) -> io::Result<()> {
        let packet = api::Message { content: Some(msg) };
        self.writer.write(&packet.encode_to_vec())?;
        Ok(())
    }
}
