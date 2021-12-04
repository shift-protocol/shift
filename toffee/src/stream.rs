use super::message::{MessageReader, MessageWriter};

pub struct DuplexStream {
    reader: MessageReader,
    writer: MessageWriter,
}
