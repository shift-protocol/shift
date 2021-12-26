use super::api;
use super::machine::Client;
use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub fn send_file(
    client: Arc<Mutex<Client>>,
    mut position: u64,
    path: &Path,
    buffer_size: usize,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let mut file = File::open(path)?;
    let size = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(position))?;
    let mut reader = BufReader::with_capacity(buffer_size, file);
    loop {
        progress(position, size);
        let length = {
            let buffer = reader.fill_buf()?;
            if buffer.len() == 0 {
                break;
            }
            client.lock().unwrap().send_chunk(api::Chunk {
                offset: position,
                data: buffer.to_vec(),
            })?;
            position += buffer.len() as u64;
            buffer.len()
        };
        reader.consume(length);
    }
    client.lock().unwrap().close_file()?;
    Ok(())
}
