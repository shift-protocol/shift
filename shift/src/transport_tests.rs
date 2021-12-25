#[cfg(test)]
use super::*;
#[cfg(test)]
use bytes::{Bytes, BytesMut};

#[test]
fn test_reader_single() {
    let mut buf = BytesMut::new();
    buf.extend("passthrough".as_bytes());
    buf.extend(TRANSPORT.prefix);
    buf.extend("dGVzdA==".as_bytes());
    buf.extend(TRANSPORT.suffix);
    buf.extend("passthrough".as_bytes());
    let mut reader = TransportReader::new(TRANSPORT);
    let result = reader.feed(&buf);
    assert_eq!(result.len(), 3);
    assert_eq!(
        result[0],
        TransportOutput::Passthrough(Bytes::from("passthrough"))
    );
    assert_eq!(result[1], TransportOutput::Packet(Bytes::from("test")));
    assert_eq!(
        result[2],
        TransportOutput::Passthrough(Bytes::from("passthrough"))
    );
}

#[test]
fn test_reader_split() {
    let mut reader = TransportReader::new(TRANSPORT);
    assert_eq!(reader.feed(TRANSPORT.prefix).len(), 0);
    assert_eq!(reader.feed("dGVzdH".as_bytes()).len(), 0);
    assert_eq!(reader.feed("Rlc3Q=".as_bytes()).len(), 0);

    let result = reader.feed(TRANSPORT.suffix);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], TransportOutput::Packet(Bytes::from("testtest")));
}

#[test]
fn test_reader_mutiple() {
    let mut buf = BytesMut::new();
    buf.extend(TRANSPORT.prefix);
    buf.extend("dGVzdA==".as_bytes());
    buf.extend(TRANSPORT.suffix);
    buf.extend(TRANSPORT.prefix);
    buf.extend("dGV".as_bytes());
    let mut reader = TransportReader::new(TRANSPORT);
    let result = reader.feed(&buf);
    assert_eq!(result[0], TransportOutput::Packet(Bytes::from("test")));

    let mut buf = BytesMut::new();
    buf.extend("zdA==".as_bytes());
    buf.extend(TRANSPORT.suffix);
    let result = reader.feed(&buf);
    assert_eq!(result[0], TransportOutput::Packet(Bytes::from("test")));
}
