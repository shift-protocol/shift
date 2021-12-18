use super::api;
use super::machine::Client;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

pub fn send_file(
    client: Arc<Mutex<Client>>,
    mut position: u64,
    path: String,
    buffer_size: usize,
    progress: &mut dyn FnMut(u64, u64),
) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let size = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(position))?;
    let mut reader = BufReader::with_capacity(buffer_size, file);
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
            progress(position, size);

            std::thread::sleep(std::time::Duration::from_millis(100));

            buffer.len()
        };
        reader.consume(length);
    }
    client.lock().unwrap().close_file().unwrap();
    Ok(())
}
