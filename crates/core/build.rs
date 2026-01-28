use std::io::Result;

fn main() -> Result<()> {
    // Compile protobuf definitions
    prost_build::compile_protos(&["proto/doli/producer.proto"], &["proto/"])?;
    Ok(())
}
