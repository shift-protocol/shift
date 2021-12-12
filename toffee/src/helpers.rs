use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use super::api;
use super::machine::Client;

pub fn send_file(client: Arc<Mutex<Client>>, mut position: u64, path: String) -> std::io::Result<()> {
    const CAP: usize = 1024 * 128;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(position))?;
    let mut reader = BufReader::with_capacity(CAP, file);
    loop {
        let length = {
            let buffer = reader.fill_buf()?;
            if buffer.len() == 0 {
                break;
            }
            client
                .lock()
                .unwrap()
                .send_chunk(api::Chunk {
                    offset: position,
                    data: buffer.to_vec(),
                })
                .unwrap();
            position += buffer.len() as u64;
            buffer.len()
        };
        reader.consume(length);
    }
    client.lock().unwrap().close_file().unwrap();
    Ok(())
}
