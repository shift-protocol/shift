use prost::Message;
use std::io;

use super::api::{self, message::Content};
use super::transport::{TransportConfig, TransportReader, TransportWriter};

pub struct MessageReader<'a> {
    reader: TransportReader<'a>,
}

impl<'a> MessageReader<'a> {
    pub fn new(
        config: TransportConfig<'a>,
        mut callback: impl FnMut(Content) -> () + Send + 'a,
    ) -> Self {
        Self {
            reader: TransportReader::new(config, move |data| {
                if let Some(msg) = api::Message::decode(data).ok() {
                    if let Some(content) = msg.content {
                        callback(content);
                    }
                }
            }),
        }
    }

    pub fn feed<'b>(&mut self, data: &'b [u8]) -> Vec<&'b [u8]> {
        self.reader.feed(data)
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
