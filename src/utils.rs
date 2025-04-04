use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use regex::Regex;

/// Gets the SHA1 hash for a given string
pub fn sha1_hex(data: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Gets the hash for a path
pub fn get_path_hash(normalized_path: &str) -> String {
    format!("@@{}", sha1_hex(normalized_path))
}

/// Ensures that the given filename has the specified extension
pub fn ensure_ext(filename: &str, ext: &str) -> String {
    let path = Path::new(filename);
    
    if let Some(extension) = path.extension() {
        if !extension.is_empty() {
            return filename.to_string();
        }
    }
    
    let ext_with_dot = if ext.starts_with('.') {
        ext.to_string()
    } else {
        format!(".{}", ext)
    };
    
    format!("{}{}", filename, ext_with_dot)
}

/// Splits a filename into name and number of keys
pub fn split_filename(file_path: &str) -> (String, Option<usize>) {
    let path = Path::new(file_path);
    let stem = path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    
    if let Some(pos) = stem.rfind('-') {
        let name = &stem[0..pos];
        let num_str = &stem[pos+1..];
        
        if let Ok(num) = num_str.parse::<usize>() {
            return (name.to_string(), Some(num));
        }
    }
    
    (stem.to_string(), None)
}

/// Remove empty lines from a string
pub fn remove_empty_lines(content: &str) -> String {
    lazy_static::lazy_static! {
        static ref EMPTY_LINE_REGEX: Regex = Regex::new(r"^\s*[\r\n]").unwrap();
    }
    
    EMPTY_LINE_REGEX.replace_all(content, "").to_string()
}

/// Replace all occurrences of a pattern in a string
pub fn replace_all(content: &str, find: &str, replace: &str) -> String {
    content.replace(find, replace)
}

/// Determines if a path is possibly mapped (starts with '~' or '<')
pub fn is_possibly_mapped_path(path: &str) -> bool {
    !path.is_empty() && (path.starts_with('~') || path.starts_with('<'))
}

/// Get the directory containing the Cargo.toml file (project root)
pub fn get_project_root() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    let mut dir = current_dir.as_path();
    
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return Some(dir.to_path_buf());
        }
        
        if let Some(parent) = dir.parent() {
            dir = parent;
        } else {
            break;
        }
    }
    
    None
}
