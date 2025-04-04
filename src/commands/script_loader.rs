use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;
use redis::Client;
use tokio::sync::RwLock;

use crate::errors::BullRsError;
use crate::mock::redis::{RedisClientExt, RedisConnectionExt};

// Type alias for any Redis client implementation (real or mock)
pub type AnyRedisClient = Box<dyn RedisClientExt + Send + Sync>;
use crate::utils::{ensure_ext, get_project_root, is_possibly_mapped_path, remove_empty_lines, replace_all, split_filename};

lazy_static! {
    static ref INCLUDE_REGEX: Regex = Regex::new(r#"/^[-]{2,3}[ \t]*@include[ \t]+(["'])(.+?)\1[; \t\n]*$/m"#).unwrap();
    static ref INCLUDE_REGEX_SINGLE: Regex = Regex::new(r#"^[-]{2,3}[ \t]*@include[ \t]+('.*?')[ \t\n;]*$"#).unwrap();
    static ref INCLUDE_REGEX_DOUBLE: Regex = Regex::new(r#"^[-]{2,3}[ \t]*@include[ \t]+(".*?")[ \t\n;]*$"#).unwrap();
}

/// Error type for script loader operations
#[derive(Debug, Clone)]
pub struct ScriptLoaderError {
    /// Error message
    pub message: String,
    /// Path of the file where the error occurred
    pub path: String,
    /// Include stack leading to the error
    pub includes: Vec<String>,
    /// Line number where the error occurred
    pub line: usize,
    /// Position in the line where the error occurred
    pub position: usize,
}

impl std::fmt::Display for ScriptLoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}:{}:{}", self.message, self.path, self.line, self.position)?;
        if !self.includes.is_empty() {
            write!(f, " (include stack: {})", self.includes.join(" -> "))?;
        }
        Ok(())
    }
}

impl std::error::Error for ScriptLoaderError {}

impl From<ScriptLoaderError> for BullRsError {
    fn from(err: ScriptLoaderError) -> Self {
        BullRsError::ScriptError(err.to_string())
    }
}

/// A representation of a file being processed
#[derive(Debug, Clone)]
pub struct FileData {
    /// The path to the file
    path: String,
    /// The content of the file
    content: String,
    /// Map of include statements: position -> included file
    includes: HashMap<usize, String>,
    /// Dependencies of this file
    dependencies: Vec<String>,
    /// Whether this file is an include or a top-level script
    is_include: bool,
}

/// A command definition
#[derive(Debug, Clone)]
pub struct CommandDefinition {
    /// Command name
    pub name: String,
    /// Number of keys this command expects
    pub num_keys: Option<usize>,
    /// Command script
    pub script: String,
    /// Lua script file path
    pub file_path: String,
}

/// Script loader with include support for Lua scripts
pub struct ScriptLoader {
    /// Path mappings for resolving script paths
    path_mapper: Mutex<HashMap<String, String>>,
    /// Cache of command definitions by directory
    command_cache: RwLock<HashMap<String, Vec<CommandDefinition>>>,
    /// Set of loaded paths by client
    client_scripts: RwLock<HashMap<usize, HashSet<String>>>,
    /// Root directory of the project
    root_path: String,
}

impl ScriptLoader {
    /// Create a new script loader instance
    pub fn new() -> Result<Self, BullRsError> {
        let root_path = get_project_root()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| String::from(""));

        let mut path_mapper = HashMap::new();
        path_mapper.insert("~".to_string(), root_path.clone());
        path_mapper.insert("rootDir".to_string(), root_path.clone());
        path_mapper.insert("base".to_string(), std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| String::from("")));

        Ok(Self {
            path_mapper: Mutex::new(path_mapper),
            command_cache: RwLock::new(HashMap::new()),
            client_scripts: RwLock::new(HashMap::new()),
            root_path,
        })
    }

    /// Add a script path mapping
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the mapping (do not include angle brackets)
    /// * `mapped_path` - The path to map to
    pub fn add_path_mapping(&self, name: &str, mapped_path: &str) -> Result<(), BullRsError> {
        let resolved = if is_possibly_mapped_path(mapped_path) {
            self.resolve_path(mapped_path, &[])?
        } else {
            // Get the current directory for relative path resolution
            let current_dir = std::env::current_dir()
                .map_err(|e| BullRsError::IoError(e))?;
            let path_buf = Path::new(mapped_path);
            let resolved_path = if path_buf.is_absolute() {
                path_buf.to_path_buf()
            } else {
                current_dir.join(path_buf)
            };
            resolved_path.to_string_lossy().to_string()
        };

        let mut path_mapper = self.path_mapper.lock().unwrap();
        path_mapper.insert(name.to_string(), resolved);
        Ok(())
    }

    /// Resolve a script path considering path mappings
    ///
    /// # Arguments
    ///
    /// * `script_name` - The name of the script
    /// * `stack` - The include stack for error reporting
    fn resolve_path(&self, script_name: &str, stack: &[String]) -> Result<String, BullRsError> {
        let mut result = script_name.to_string();

        // Handle paths with ~ prefix (root path)
        if script_name.starts_with('~') {
            result = format!("{}{}", self.root_path, &script_name[1..]);
        } 
        // Handle paths with explicit mapping using <mapping>
        else if script_name.starts_with('<') {
            if let Some(p) = script_name.find('>') {
                let name = &script_name[1..p];
                let path_mapper = self.path_mapper.lock().unwrap();
                
                let mapped_path = path_mapper.get(name).cloned();
                if let Some(mapped) = mapped_path {
                    result = format!("{}{}", mapped, &script_name[p+1..]);
                } else {
                    return Err(BullRsError::PathMappingNotFoundError(name.to_string()));
                }
            }
        }
        // Handle paths that might be mapped directly (without < > syntax)
        else if is_possibly_mapped_path(script_name) {
            // Split path into possible mapping name and the rest of the path
            if let Some(separator_pos) = script_name.find('/') {
                let mapping_name = &script_name[0..separator_pos];
                let path_suffix = &script_name[separator_pos..];
                
                let path_mapper = self.path_mapper.lock().unwrap();
                if let Some(mapped_path) = path_mapper.get(mapping_name).cloned() {
                    result = format!("{}{}", mapped_path, path_suffix);
                }
                // If not found, we'll use the original path as fallback
            }
        }

        let path = Path::new(&result);
        Ok(path.to_string_lossy().to_string())
    }

    /// Recursively collect all scripts included in a file
    ///
    /// # Arguments
    ///
    /// * `file` - The parent file
    /// * `cache` - A cache for file metadata
    /// * `is_include` - Whether this file is an include
    /// * `stack` - Include stack for circular reference detection
    async fn resolve_dependencies(
        &self,
        file_path: &str,
        cache: &mut HashMap<String, FileData>,
        is_include: bool,
        stack: &mut Vec<String>,
    ) -> Result<FileData, BullRsError> {
        // Check for circular references
        if stack.contains(&file_path.to_string()) {
            return Err(BullRsError::CircularReferenceError(file_path.to_string()));
        }
        stack.push(file_path.to_string());

        // Return cached data if available
        if let Some(file_data) = cache.get(file_path) {
            stack.pop();
            return Ok(file_data.clone());
        }

        // Read file content
        let content = fs::read_to_string(file_path)
            .map_err(|e| BullRsError::IoError(e))?;

        let mut file_data = FileData {
            path: file_path.to_string(),
            content,
            includes: HashMap::new(),
            dependencies: Vec::new(),
            is_include,
        };

        // Find all include directives
        for (_line_idx, line) in file_data.content.lines().enumerate() {
            if let Some(captures) = INCLUDE_REGEX_SINGLE.captures(line) {
                if let Some(path_match) = captures.get(1) {
                    let path = path_match.as_str();
                    // Remove the quotes (first and last character)
                    let path_without_quotes = &path[1..path.len()-1];
                    // Process the included file
                    let resolved_path = self.resolve_path(path_without_quotes, stack)?;
                    let resolved_path = ensure_ext(&resolved_path, "lua");
                    
                    // Store the include statement's position
                    let pos = file_data.content.find(line).unwrap_or(0);
                    file_data.includes.insert(pos, path_without_quotes.to_string());
                    
                    // Process the included file - use Box::pin to avoid infinite future size
                    let included_file = Box::pin(self.resolve_dependencies(
                        &resolved_path,
                        cache,
                        true,
                        stack,
                    )).await?;
                    
                    // Add the included file and its dependencies to our dependencies
                    file_data.dependencies.push(resolved_path.clone());
                    file_data.dependencies.extend(included_file.dependencies.clone());
                }
            } else if let Some(captures) = INCLUDE_REGEX_DOUBLE.captures(line) {
                if let Some(path_match) = captures.get(1) {
                    let path = path_match.as_str();
                    // Remove the quotes (first and last character)
                    let path_without_quotes = &path[1..path.len()-1];
                    // Process the included file
                    let resolved_path = self.resolve_path(path_without_quotes, stack)?;
                    let resolved_path = ensure_ext(&resolved_path, "lua");
                    
                    // Store the include statement's position
                    let pos = file_data.content.find(line).unwrap_or(0);
                    file_data.includes.insert(pos, path_without_quotes.to_string());
                    
                    // Process the included file - use Box::pin to avoid infinite future size
                    let included_file = Box::pin(self.resolve_dependencies(
                        &resolved_path,
                        cache,
                        true,
                        stack,
                    )).await?;
                    
                    // Add the included file and its dependencies to our dependencies
                    file_data.dependencies.push(resolved_path.clone());
                    file_data.dependencies.extend(included_file.dependencies.clone());
                }
            }
        }

        // Add this file to the cache
        cache.insert(file_path.to_string(), file_data.clone());
        
        // Pop this file from the stack before returning
        stack.pop();
        
        Ok(file_data)
    }

    /// Parse a Lua script
    ///
    /// # Arguments
    ///
    /// * `filename` - The file path
    /// * `content` - The content of the script
    /// * `cache` - Cache for parsed files
    async fn parse_script(
        &self,
        filename: &str,
        content: &str,
        cache: &mut HashMap<String, FileData>,
    ) -> Result<FileData, BullRsError> {
        let mut stack = Vec::new();
        let file_data = self.resolve_dependencies(filename, cache, false, &mut stack).await?;
        Ok(file_data)
    }

    /// Interpolate includes in a file's content
    ///
    /// # Arguments
    ///
    /// * `file` - The file to interpolate
    /// * `processed` - Set of processed includes to avoid duplicates
    fn interpolate(
        &self,
        file: &FileData,
        processed: &mut HashSet<String>,
    ) -> Result<String, BullRsError> {
        let mut result = file.content.clone();
        
        // For each include, replace the include statement with the file content
        for (pos, include_path) in &file.includes {
            let resolved_path = self.resolve_path(include_path, &[])?;
            let resolved_path = ensure_ext(&resolved_path, "lua");
            
            // Skip if this include was already processed
            if processed.contains(&resolved_path) {
                let placeholder = format!("-- @include '{}' (already included)", include_path);
                result = replace_all(&result, &file.content[*pos..(*pos+include_path.len()+12)], &placeholder);
                continue;
            }
            
            // Mark this include as processed
            processed.insert(resolved_path.clone());
            
            // Get the include's content from the cache
            let included_content = fs::read_to_string(&resolved_path)
                .map_err(|e| BullRsError::IoError(e))?;
            
            // Replace the include directive with the file content
            let line_with_include = file.content.lines()
                .nth(*pos)
                .unwrap_or("");
            
            result = replace_all(&result, line_with_include, &included_content);
        }
        
        Ok(remove_empty_lines(&result))
    }

    /// Load a command from a Lua script file
    ///
    /// # Arguments
    ///
    /// * `filename` - The file path
    /// * `cache` - Cache for parsed files
    async fn load_command(
        &self,
        filename: &str,
        cache: &mut HashMap<String, FileData>,
    ) -> Result<CommandDefinition, BullRsError> {
        let content = fs::read_to_string(filename)
            .map_err(|e| BullRsError::IoError(e))?;
        
        let file_data = self.parse_script(filename, &content, cache).await?;
        let mut processed = HashSet::new();
        let script = self.interpolate(&file_data, &mut processed)?;
        
        let (name, num_keys) = split_filename(filename);
        
        Ok(CommandDefinition {
            name,
            num_keys,
            script,
            file_path: filename.to_string(),
        })
    }

    /// Load all Lua scripts in a directory
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory containing Lua scripts
    /// * `cache` - Cache for parsed files
    pub async fn load_scripts(
        &self,
        dir: Option<&str>,
        cache: Option<&mut HashMap<String, FileData>>,
    ) -> Result<Vec<CommandDefinition>, BullRsError> {
        let dir_path = dir.unwrap_or(".");
        let dir_path = Path::new(dir_path);
        let dir_str = dir_path.to_string_lossy().to_string();
        
        // Check if commands for this directory are already cached
        {
            let command_cache = self.command_cache.read().await;
            if let Some(commands) = command_cache.get(&dir_str) {
                return Ok(commands.clone());
            }
        }
        
        // Read directory entries
        let entries = fs::read_dir(dir_path)
            .map_err(|e| BullRsError::IoError(e))?;
        
        // Filter for Lua files
        let mut lua_files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| BullRsError::IoError(e))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
                lua_files.push(path);
            }
        }
        
        if lua_files.is_empty() {
            return Err(BullRsError::GeneralError("No .lua files found!".to_string()));
        }
        
        // Load each command
        let mut commands = Vec::new();
        let mut local_cache = HashMap::new();
        let cache = cache.unwrap_or(&mut local_cache);
        
        for file_path in lua_files {
            let command = self.load_command(
                &file_path.to_string_lossy(),
                cache,
            ).await?;
            commands.push(command);
        }
        
        // Cache the commands
        {
            let mut command_cache = self.command_cache.write().await;
            command_cache.insert(dir_str, commands.clone());
        }
        
        Ok(commands)
    }

    /// Attach Lua scripts to a Redis client
    ///
    /// # Arguments
    ///
    /// * `client` - Redis client
    /// * `pathname` - Directory containing Lua scripts
    /// * `cache` - Cache for parsed files
    pub async fn load(
        &self,
        client: &dyn RedisClientExt,
        pathname: &str,
        cache: Option<&mut HashMap<String, FileData>>,
    ) -> Result<(), BullRsError> {
        // Cast through a specific type first to get a thin pointer for trait objects
        let client_ptr = client as *const dyn RedisClientExt;
        let client_id = client_ptr as *const () as usize;
        
        // Check if scripts for this client and pathname are already loaded
        let mut load_needed = false;
        {
            let client_scripts = self.client_scripts.read().await;
            if let Some(paths) = client_scripts.get(&client_id) {
                if !paths.contains(pathname) {
                    load_needed = true;
                }
            } else {
                load_needed = true;
            }
        }
        
        if load_needed {
            // Mark this pathname as loaded for this client
            {
                let mut client_scripts = self.client_scripts.write().await;
                let paths = client_scripts.entry(client_id).or_insert_with(HashSet::new);
                paths.insert(pathname.to_string());
            }
            
            // Load the scripts
            let scripts = self.load_scripts(Some(pathname), cache).await?;
            
            // Register each script with the Redis client
            let mut conn = client.get_connection()
                .map_err(|e| BullRsError::RedisError(e))?;
            
            println!("Loaded {} scripts, registering with Redis", scripts.len());
            
            for command in scripts {
                let script = command.script.clone();
                let num_keys = command.num_keys.unwrap_or(0);
                
                println!("Loading script '{}' ({}kb) into Redis", 
                         command.name, 
                         script.len() / 1024);
                
                // Try to load the script, with better error handling
                match conn.execute_command("SCRIPT", &["LOAD", &script]) {
                    Ok(redis::Value::Data(data)) => {
                        // Convert bytes to string for the SHA
                        let script_sha = String::from_utf8_lossy(&data).to_string();
                        println!("Successfully loaded script: {} with SHA: {}", command.name, script_sha);
                    },
                    Ok(_) => {
                        println!("Unexpected response from Redis when loading script");
                    },
                    Err(e) => {
                        println!("Error loading script into Redis: {}", e);
                        return Err(BullRsError::RedisError(e));
                    }
                }
            }
        }
        
        Ok(())
    }

    /// Clear the command cache
    pub async fn clear_cache(&self) {
        let mut command_cache = self.command_cache.write().await;
        command_cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_resolve_path() {
        let loader = ScriptLoader::new().unwrap();
        
        // Test normal path
        let path = loader.resolve_path("test.lua", &[]).unwrap();
        assert_eq!(path, "test.lua");
        
        // Test with mapped path
        loader.add_path_mapping("test", "/tmp").unwrap();
        let path = loader.resolve_path("<test>/script.lua", &[]).unwrap();
        assert_eq!(path, "/tmp/script.lua");
    }
}
