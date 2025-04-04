use std::env;
use std::path::Path;

use anyhow::Result;
use redis::Client;
use tokio;

use bullrs::mock::redis::{MockRedisClient, RedisClientExt};

use bullrs::ScriptLoader;
use bullrs::errors::BullRsError;

#[tokio::main]
async fn main() -> Result<(), BullRsError> {
    // Initialize logging
    env_logger::init();

    println!("BullRs - Bull Queue in Rust");
    
    // Create a new script loader
    let script_loader = ScriptLoader::new()?;
    
    // Add path mappings for script includes
    let current_dir = env::current_dir()?;
    let src_path = current_dir.join("src");
    let lib_commands_path = src_path.join("lib").join("commands");
    let lib_includes_path = lib_commands_path.join("includes");
    
    // Configure the script loader
    println!("Commands path: {}", lib_commands_path.display());
    println!("Includes path: {}", lib_includes_path.display());
    
    let includes_file = lib_includes_path.join("removeDebounceKey.lua");
    println!("Include file exists: {}", includes_file.display());
    
    // Create absolute paths for all directories
    let commands_abs_path = lib_commands_path.canonicalize()
        .map_err(|e| BullRsError::IoError(e))?
        .to_string_lossy()
        .to_string();
    
    let includes_abs_path = lib_includes_path.canonicalize()
        .map_err(|e| BullRsError::IoError(e))?
        .to_string_lossy()
        .to_string();
    
    println!("Commands absolute path: {}", commands_abs_path);
    println!("Includes absolute path: {}", includes_abs_path);
    
    // Set up mappings for script includes with absolute paths
    script_loader.add_path_mapping("commands", &commands_abs_path)?;
    script_loader.add_path_mapping("includes", &includes_abs_path)?;
    
    // Add relative mappings as well to handle multiple reference styles
    script_loader.add_path_mapping("../includes", &includes_abs_path)?;
    script_loader.add_path_mapping("./includes", &includes_abs_path)?;
    script_loader.add_path_mapping("includes", &includes_abs_path)?;
    
    // Special mapping for the main includes file that's referenced in code
    script_loader.add_path_mapping("removeDebounceKey", &format!("{}/removeDebounceKey.lua", includes_abs_path))?;
    
    println!("Path mappings configured successfully");
    
    // To debug path resolution issues, let's verify a key path exists
    let test_include_path = lib_includes_path.join("removeDebounceKey.lua");
    if test_include_path.exists() {
        println!("Include file exists: {}", test_include_path.display());
    } else {
        println!("Warning: Include file not found: {}", test_include_path.display());
    }
    
    // Try to connect to Redis or fall back to mock for testing
    let redis_url = "redis://127.0.0.1:6379";
    println!("Attempting to connect to Redis at: {}", redis_url);
    
    // Get a Redis client (real or mock)
    let client: Box<dyn RedisClientExt> = match Client::open(redis_url) {
        Ok(real_client) => {
            println!("Successfully connected to Redis");
            Box::new(real_client)
        },
        Err(e) => {
            println!("IMPORTANT: Could not connect to Redis: {}", e);
            println!("Using mock Redis client for testing purposes");
            Box::new(MockRedisClient::new())
        }
    };
    
    // Verify paths exist before loading scripts
    let commands_dir = lib_commands_path.to_str().unwrap();

    // Check if the directory exists and can be read
    if !lib_commands_path.exists() {
        println!("Warning: Commands directory does not exist: {}", commands_dir);
        // Create directory if it doesn't exist
        println!("Creating directory: {}", commands_dir);
        std::fs::create_dir_all(&lib_commands_path)
            .map_err(|e| BullRsError::IoError(e))?
    }

    // List files in the commands directory to verify access
    println!("Files in commands directory:");
    let mut lua_files = Vec::new();
    for entry in std::fs::read_dir(&lib_commands_path).map_err(|e| BullRsError::IoError(e))? {
        let entry = entry.map_err(|e| BullRsError::IoError(e))?;
        let path = entry.path();
        println!("  - {}", path.display());
        
        // Check for Lua scripts
        if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            lua_files.push(path);
        }
    }
    
    println!("\nFound {} Lua script files", lua_files.len());
    if lua_files.is_empty() {
        println!("Warning: No Lua script files found in {}", commands_dir);
    } else {
        println!("First few Lua files:");
        for file in lua_files.iter().take(3) {
            println!("  - {}", file.display());
            
            // Verify file can be read
            match std::fs::read_to_string(file) {
                Ok(content) => {
                    println!("    File is readable: {} bytes", content.len());
                    // Print first few lines
                    let lines: Vec<&str> = content.lines().take(3).collect();
                    println!("    First few lines: {}", lines.join(" | "));
                },
                Err(e) => println!("    Error reading file: {}", e)
            }
        }
    }

    println!("Loading scripts from: {}", commands_dir);

    // Use an absolute path to avoid any path resolution issues
    let absolute_commands_dir = lib_commands_path.canonicalize()
        .map_err(|e| BullRsError::IoError(e))?;
    let absolute_commands_path = absolute_commands_dir.to_str().unwrap();
    println!("Absolute command path: {}", absolute_commands_path);

    // Load scripts into Redis or mock with detailed error reporting
    println!("Starting script loading with detailed debugging...");
    match script_loader.load(client.as_ref(), absolute_commands_path, None).await {
        Ok(_) => println!("Successfully loaded Lua scripts"),
        Err(e) => {
            println!("Failed to load scripts: {}", e);
            if let BullRsError::IoError(io_err) = &e {
                println!("IO Error details: Kind={:?}, Message={}", io_err.kind(), io_err);
                
                // Try to load scripts individually to find which one fails
                println!("\nAttempting to identify problematic script...");
                for entry in std::fs::read_dir(&lib_commands_path).unwrap() {
                    let entry = entry.unwrap();
                    let path = entry.path();
                    
                    // Skip directories and non-lua files
                    if path.is_dir() || path.extension().and_then(|ext| ext.to_str()) != Some("lua") {
                        continue;
                    }
                    
                    println!("Checking script: {}", path.display());
                    
                    // Try to read the file and perform include resolution
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            println!("  Successfully read file: {} ({} bytes)", path.display(), content.len());
                            
                            // Look for include statements
                            if content.contains("--include") || content.contains("--@include") {
                                println!("  Contains include statements, checking them:");
                                let includes = find_includes(&content);
                                for include_path in includes {
                                    println!("    Include: {}", include_path);
                                    
                                    // Try to resolve include path relative to the command directory
                                    let full_include_path = lib_commands_path.join("includes").join(&include_path);
                                    println!("    Full include path: {}", full_include_path.display());
                                    
                                    // Check if the include file exists
                                    if !full_include_path.exists() {
                                        println!("    ERROR: Include file does not exist: {}", full_include_path.display());
                                    } else {
                                        println!("    Include file exists: {}", full_include_path.display());
                                    }
                                }
                            }
                        },
                        Err(err) => println!("  ERROR reading file: {}, Error: {}", path.display(), err)
                    }
                }
            }
        },
    }
    
    // Helper function to find include statements in Lua scripts
    fn find_includes(content: &str) -> Vec<String> {
        let mut includes = Vec::new();
        for line in content.lines() {
            // Match --include('file.lua') or similar patterns
            if let Some(pos) = line.find("--include(") {
                if let Some(end_pos) = line[pos..].find(")") {
                    let include_expr = &line[pos + 10..pos + end_pos];
                    let include_path = include_expr.trim_matches('\'')
                                                  .trim_matches('"');
                    includes.push(include_path.to_string());
                }
            }
            // Match --@include('file.lua') or similar patterns
            else if let Some(pos) = line.find("--@include(") {
                if let Some(end_pos) = line[pos..].find(")") {
                    let include_expr = &line[pos + 11..pos + end_pos];
                    let include_path = include_expr.trim_matches('\'')
                                                  .trim_matches('"');
                    includes.push(include_path.to_string());
                }
            }
        }
        includes
    }
    
    Ok(())
}
