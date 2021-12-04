use twoway;

use super::constants::*;

pub struct TransportReader<F>
where F: Fn(&[u8]) -> () {
    buffer: Vec<u8>,
    callback: F,
    in_sequence: bool,
}

impl<F> TransportReader<F>
where F: Fn(&[u8]) -> () {
    pub fn new (callback: F) -> Self {
        Self { buffer: vec![], callback, in_sequence: false }
    }

    pub fn feed<'a> (&mut self, data: &'a[u8]) -> Vec<&'a[u8]> {
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
                    break
                }
            }
        }

        views
    }

    fn feed_remainder<'a> (&mut self, data: &'a[u8]) -> &'a[u8] {
        match twoway::find_bytes(&data, SUFFIX) {
            Some(length) => {
                let packet = &data[..length];
                (self.callback)(packet);
                self.in_sequence = false;
                return &data[length + SUFFIX.len()..];
            }
            None => {
                self.in_sequence = true;
                self.buffer.extend_from_slice(&data);
                return &[]
            }
        }
    }
}
