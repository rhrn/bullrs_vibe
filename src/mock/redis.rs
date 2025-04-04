use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use redis::{Client, Connection, RedisError, RedisResult, Value};

/// A mock Redis client for testing purposes.
/// This allows running tests without needing a real Redis server.
#[derive(Clone)]
pub struct MockRedisClient {
    /// Store loaded scripts with their SHA1 hashes
    scripts: Arc<Mutex<HashMap<String, String>>>,
    /// Store key-value pairs
    data: Arc<Mutex<HashMap<String, Value>>>,
}

impl MockRedisClient {
    /// Create a new mock Redis client
    pub fn new() -> Self {
        Self {
            scripts: Arc::new(Mutex::new(HashMap::new())),
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Mock implementation of the SCRIPT LOAD command
    pub fn script_load(&self, script: &str) -> RedisResult<String> {
        // In a real Redis server, this would compute the SHA1 hash of the script
        // For our mock, we'll use a simple hash function
        let script_hash = format!("{:x}", md5::compute(script));
        
        // Store the script with its hash
        let mut scripts = self.scripts.lock().unwrap();
        scripts.insert(script_hash.clone(), script.to_string());
        
        Ok(script_hash)
    }
    
    /// Get a mock connection
    pub fn get_mock_connection(&self) -> MockRedisConnection {
        MockRedisConnection {
            client: self.clone(),
        }
    }
}

// Implement conversion from MockRedisClient to the real Redis Client for compatibility
impl From<MockRedisClient> for Client {
    fn from(_mock: MockRedisClient) -> Self {
        // This is a stub implementation - in real code, we would never
        // actually convert a mock to a real client
        Client::open("redis://localhost:6379").unwrap()
    }
}

/// A mock Redis connection for testing purposes
pub struct MockRedisConnection {
    client: MockRedisClient,
}

impl MockRedisConnection {
    /// Handle Redis commands in the mock
    pub fn execute_command(&mut self, cmd: &str, args: &[&str]) -> RedisResult<Value> {
        match cmd.to_uppercase().as_str() {
            "SCRIPT" => {
                if args.is_empty() {
                    return Err(RedisError::from((redis::ErrorKind::IoError, "Invalid SCRIPT command")));
                }
                
                match args[0].to_uppercase().as_str() {
                    "LOAD" => {
                        if args.len() < 2 {
                            return Err(RedisError::from((redis::ErrorKind::IoError, "Missing script argument")));
                        }
                        
                        let script = args[1];
                        let hash = self.client.script_load(script)?;
                        Ok(Value::Data(hash.into_bytes()))
                    },
                    _ => Err(RedisError::from((redis::ErrorKind::IoError, "Unsupported SCRIPT subcommand")))
                }
            },
            _ => Err(RedisError::from((redis::ErrorKind::IoError, "Unsupported command")))
        }
    }
}

// Extension trait to allow easy switching between real and mock clients
pub trait RedisClientExt {
    fn is_mock(&self) -> bool;
    fn get_connection(&self) -> RedisResult<Box<dyn RedisConnectionExt>>;
}

// Extension trait for Redis connections
pub trait RedisConnectionExt {
    fn execute_command(&mut self, cmd: &str, args: &[&str]) -> RedisResult<Value>;
}

// Implement the extension trait for the mock client
impl RedisClientExt for MockRedisClient {
    fn is_mock(&self) -> bool {
        true
    }
    
    fn get_connection(&self) -> RedisResult<Box<dyn RedisConnectionExt>> {
        Ok(Box::new(self.get_mock_connection()))
    }
}

// Implement the extension trait for the real Redis client
impl RedisClientExt for Client {
    fn is_mock(&self) -> bool {
        false
    }
    
    fn get_connection(&self) -> RedisResult<Box<dyn RedisConnectionExt>> {
        let conn = self.get_connection()?;
        Ok(Box::new(RealRedisConnection(conn)))
    }
}

// Wrapper for real Redis connection
pub struct RealRedisConnection(Connection);

impl RedisConnectionExt for RealRedisConnection {
    fn execute_command(&mut self, cmd: &str, args: &[&str]) -> RedisResult<Value> {
        let mut redis_cmd = redis::cmd(cmd);
        for arg in args {
            redis_cmd.arg(*arg);
        }
        redis_cmd.query(&mut self.0)
    }
}

// Implement the connection extension trait for the mock connection
impl RedisConnectionExt for MockRedisConnection {
    fn execute_command(&mut self, cmd: &str, args: &[&str]) -> RedisResult<Value> {
        self.execute_command(cmd, args)
    }
}
