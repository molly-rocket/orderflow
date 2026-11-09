use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new("proto");
    let files = ["orderflow/v1/market_data.proto"];
    println!("cargo:rerun-if-changed={}", root.display());

    let descriptors = protox::compile(files, [root])?;
    prost_build::Config::new()
        .enable_type_names()
        .compile_fds(descriptors)?;
    Ok(())
}
