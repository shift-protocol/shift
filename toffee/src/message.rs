use bytes::Bytes;
use cancellation::*;
use prost::Message;
use std::io::{self, Read};

use super::api::{self, message::Content};
use super::transport::{
    TransportConfig, TransportFeeder, TransportOutput, TransportReader, TransportWriter,
};

fn decode_from(output: Vec<TransportOutput>) -> Vec<MessageOutput> {
    let mut result = vec![];
    for part in output {
        match part {
            TransportOutput::Passthrough(data) => result.push(MessageOutput::Passthrough(data)),
            TransportOutput::Packet(data) => {
                if let Some(msg) = api::Message::decode(data).ok() {
                    if let Some(content) = msg.content {
                        result.push(MessageOutput::Message(content));
                    }
                }
            }
        }
    }
    result
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
        println!("1");
        Self {
            feeder: reader.feed_from(stream, ct),
        }
    }
}

impl<'a> Iterator for MessageFeeder<'a> {
    type Item = MessageOutput;

    fn next(&mut self) -> Option<MessageOutput> {
        match self.feeder.next() {
            None => return None,
            Some(item) => {
                for output in decode_from(vec![item]) {
                    return Some(output);
                }
                None
            }
        }
    }
}

impl<'a> MessageReader<'a> {
    pub fn new(config: TransportConfig<'a>) -> Self {
        Self {
            reader: TransportReader::new(config),
        }
    }

    pub fn feed<'b>(&mut self, data: &'b [u8]) -> Vec<MessageOutput> {
        let output = self.reader.feed(data);
        decode_from(output)
    }

    pub fn feed_from(
        &'a mut self,
        stream: &'a mut dyn Read,
        ct: &'a CancellationToken,
    ) -> MessageFeeder<'a> {
        return MessageFeeder::new(&mut self.reader, stream, ct);
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
