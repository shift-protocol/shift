use super::transport::TransportConfig;

pub const SERVER_TRANSPORT: TransportConfig = TransportConfig {
    prefix: "\x1b]1337;Toffee;S;".as_bytes(),
    suffix: "\x07".as_bytes(),
};

pub const CLIENT_TRANSPORT: TransportConfig = TransportConfig {
    prefix: "\x1b]1337;Toffee;C;".as_bytes(),
    suffix: "\x07".as_bytes(),
};
