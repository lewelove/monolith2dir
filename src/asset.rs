use anyhow::{Context, Result};
use data_url::DataUrl;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct AssetStore {
    assets_dir: PathBuf,
    assets_dir_name: String,
    dedup: HashMap<[u8; 32], String>,
}

impl AssetStore {
    pub fn new<P: AsRef<Path>>(output_dir: P, assets_dir_name: &str) -> Result<Self> {
        let assets_dir = output_dir.as_ref().join(assets_dir_name);
        fs::create_dir_all(&assets_dir).with_context(|| {
            format!(
                "Failed to create assets directory: {}",
                assets_dir.display()
            )
        })?;

        Ok(Self {
            assets_dir,
            assets_dir_name: assets_dir_name.to_string(),
            dedup: HashMap::new(),
        })
    }

    pub fn process_bytes(&mut self, bytes: &[u8], ext: &str) -> Option<String> {
        if bytes.is_empty() {
            return None;
        }

        let hash = blake3::hash(bytes);
        let hash_bytes = *hash.as_bytes();
        let hash_hex = hash.to_hex();

        if let Some(rel_path) = self.dedup.get(&hash_bytes) {
            return Some(rel_path.clone());
        }

        let filename = format!("{hash_hex}.{ext}");
        let file_path = self.assets_dir.join(&filename);

        if !file_path.exists() {
            fs::write(&file_path, bytes).ok()?;
        }

        let rel_path = format!("{}/{}", self.assets_dir_name, filename);
        self.dedup.insert(hash_bytes, rel_path.clone());
        Some(rel_path)
    }

    pub fn process_data_url(&mut self, raw_url: &str) -> Option<String> {
        let trimmed = raw_url.trim();
        let data_url = DataUrl::process(trimmed).ok()?;
        let (body, _) = data_url.decode_to_vec().ok()?;
        if body.is_empty() {
            return None;
        }

        let mime = data_url.mime_type();
        let ext = resolve_extension(&mime.type_, &mime.subtype);
        self.process_bytes(&body, ext)
    }

    #[must_use]
    pub fn asset_count(&self) -> usize {
        self.dedup.len()
    }
}

#[must_use]
pub fn resolve_extension(top_level: &str, subtype: &str) -> &'static str {
    let mime_str = format!("{top_level}/{subtype}");
    if let Some(first) = mime_guess::get_mime_extensions_str(&mime_str)
        .and_then(|exts| exts.first().copied())
    {
        return first;
    }
    match subtype {
        "svg+xml" => "svg",
        "x-icon" | "vnd.microsoft.icon" => "ico",
        "webp" => "webp",
        "jpeg" => "jpg",
        "png" => "png",
        "gif" => "gif",
        "css" => "css",
        "javascript" | "x-javascript" => "js",
        "woff2" => "woff2",
        "woff" => "woff",
        "ttf" => "ttf",
        "otf" => "otf",
        _ => "bin",
    }
}
