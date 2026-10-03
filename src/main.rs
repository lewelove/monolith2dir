use anyhow::{Context, Result};
use clap::Parser;
use std::cell::RefCell;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use monolith2dir::{AssetStore, HtmlUnbundler};

const ASSETS_DIR: &str = "assets";
const SCRIPTS_DIR: &str = "scripts";

#[derive(Parser, Debug)]
#[command(
    name = "monolith2dir",
    version,
    about = "Unpack self-contained Monolith HTML into index.html and assets directory"
)]
struct Args {
    #[arg(help = "Path to input HTML file (use '-' for stdin)")]
    input: String,

    #[arg(short, long, help = "Output directory")]
    output: Option<PathBuf>,

    #[arg(
        long,
        default_value = "0",
        help = "Minimum byte size of inline scripts to extract"
    )]
    min_script_bytes: u32,

    #[arg(short, long, help = "Overwrite output directory if it exists")]
    force: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let (html_content, default_out_dir) = if args.input == "-" {
        let mut buffer = String::new();
        io::stdin()
            .read_to_string(&mut buffer)
            .context("Failed to read HTML from stdin")?;
        (buffer, PathBuf::from("unbundled_output"))
    } else {
        let input_path = Path::new(&args.input);
        let content = fs::read_to_string(input_path).with_context(|| {
            format!("Failed to read input file: {}", input_path.display())
        })?;
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        (content, PathBuf::from(stem))
    };

    let output_dir = args.output.unwrap_or(default_out_dir);

    if output_dir.exists() && !args.force && fs::read_dir(&output_dir)?.next().is_some() {
        anyhow::bail!(
            "Output directory {} already exists and is not empty. Use --force to proceed.",
            output_dir.display()
        );
    }

    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "Failed to create output directory: {}",
            output_dir.display()
        )
    })?;

    let asset_store = Rc::new(RefCell::new(AssetStore::new(&output_dir, ASSETS_DIR)?));
    let script_store = Rc::new(RefCell::new(AssetStore::new(&output_dir, SCRIPTS_DIR)?));
    let unbundler = HtmlUnbundler::new(
        Rc::clone(&asset_store),
        Rc::clone(&script_store),
        args.min_script_bytes,
    );

    let clean_html = unbundler.unbundle(&html_content)?;

    let index_path = output_dir.join("index.html");
    fs::write(&index_path, clean_html).with_context(|| {
        format!("Failed to write index file: {}", index_path.display())
    })?;

    let asset_count = asset_store.borrow().asset_count();
    let script_count = script_store.borrow().asset_count();
    eprintln!(
        "Extracted {asset_count} unique assets into {}/{ASSETS_DIR}",
        output_dir.display()
    );
    eprintln!(
        "Extracted {script_count} unique scripts into {}/{SCRIPTS_DIR}",
        output_dir.display()
    );
    eprintln!("Saved clean HTML to {}", index_path.display());

    Ok(())
}
