use super::transport::TransportConfig;

pub const TRANSPORT: TransportConfig = TransportConfig {
    prefix: "\x1b]1337;Toffee;".as_bytes(),
    suffix: "\x07".as_bytes(),
};
