use std::{path::PathBuf, fs, io::{Read, Write}};

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;

#[derive(clap::Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// In dir to read the wasm and js to transform
    #[arg(short, long)]
    in_dir: PathBuf,

    /// Out dir to write the transformed wasm and js to
    #[arg(short, long)]
    out_dir: PathBuf,

    /// Name of the package to transform
    #[arg(short, long)]
    pkg_name: String
}

fn main() -> Result<()> {
    pretty_env_logger::init();

    // Parse app arguments
    let args = Args::parse();

    let pkg_folder = &args.in_dir;
    let pkg_name = &args.pkg_name;
    let destination = &args.out_dir;

    let wasm_name = pkg_name.to_string() + "_bg.wasm";
    let js_name = pkg_name.to_string() + ".js";

    // Create destination folder in case it didn't exist
    fs::create_dir_all(destination)?;

    // Read wasm and use walrus to change memory parameters and save back to out_dir
    let wasm = pkg_folder.join(&wasm_name);
    let mut module = walrus::Module::from_file(&wasm).context("Opening wasm")?;
    for mem in module.memories.iter_mut() {
        log::info!("{:?} shared {}", mem.name, mem.shared);
        mem.initial = 100;
        mem.maximum = Some(16384);
        mem.shared = true;
    }
    let wasm = module.emit_wasm();

    let wasm_destination = destination.join(&wasm_name);
    std::fs::write(wasm_destination, wasm).context("Writing modified wasm")?;

    // Read js glue source and replace needed bits using regex
    // We need to change the initialization of the memory to use shared memory but also certain
    // functions to pass memory back and forth between js/wasm which need to be changed when
    // using shared memory
    let js_path = pkg_folder.join(&js_name);
    let mut js_file = fs::File::open(js_path)?;
    let mut js_source = String::new();
    js_file.read_to_string(&mut js_source)?;

    // Use shared memory
    let regex = Regex::new("imports\\.wbg\\.memory = maybe_memory \\|\\| new WebAssembly\\.Memory\\(\\{initial:(?<initial>[0-9]+)\\}\\);")?;
    let mut new_js_source = regex.replace(
        &js_source,
        "imports.wbg.memory = maybe_memory || new WebAssembly.Memory({initial: 100, maximum: 16384, shared: true });"
    ).to_string();

    // TextDecoder can't use shared memory so we need to slice rather than subarray
    new_js_source = new_js_source.replace(
        "return cachedTextDecoder.decode(getUint8Memory0().subarray(ptr, ptr + len));",
        "return cachedTextDecoder.decode(getUint8Memory0().slice(ptr, ptr + len));"
    );

    // all the memory getters have to change from checking byteLength != 0 to checking that the
    // memory buffer is different than the wasm linear memory one
    let regex_memory = Regex::new(
        "if \\(cached(?<memory_type_lhs>.*)Memory(?<memory_id_lhs>[0-9]+) === null \\|\\| cached(?<memory_type_rhs>.*)Memory(?<memory_id_rhs>[0-9]+).byteLength === 0\\) \\{",
    )?;
    let new_js_source = regex_memory.replace_all(
        &new_js_source,
        "if (cached${memory_type_lhs}Memory${memory_id_lhs} === null || cached${memory_type_rhs}Memory${memory_id_rhs}.buffer !== wasm.memory.buffer) {"
    );

    let mut destination_js = fs::File::create(destination.join(&js_name))?;
    destination_js.write_all(new_js_source.as_bytes())?;

    Ok(())
}