use std::io::Result;

fn main() -> Result<()> {
    prost_build::compile_protos(&["proto/file_transfer.proto"], &["proto/"])?;
    Ok(())
}
