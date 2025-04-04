use thiserror::Error;

/// Custom error types for BullRs
#[derive(Error, Debug)]
pub enum BullRsError {
    /// Error related to Redis operations
    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),
    
    /// Error related to IO operations
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    /// Error related to script loading or execution
    #[error("Script error: {0}")]
    ScriptError(String),

    /// Error when parsing lua scripts
    #[error("Parsing error: {0}")]
    ParsingError(String),
    
    /// Error for circular references
    #[error("Circular reference: {0}")]
    CircularReferenceError(String),
    
    /// Error when a path mapping is not found
    #[error("No path mapping found for {0}")]
    PathMappingNotFoundError(String),
    
    /// Error related to serialization or deserialization
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    /// Error related to process execution
    #[error("Process error: {0}")]
    ProcessError(String),
    
    /// Error when a process times out
    #[error("Process timeout")]
    ProcessTimeout,
    
    /// Error when maximum process limit is reached
    #[error("Maximum process limit reached")]
    ProcessLimitReached,
    
    /// Error related to system time operations
    #[error("System time error: {0}")]
    SystemTimeError(String),
    
    /// Error when an invalid argument is provided
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    
    /// Error when a file is not found
    #[error("File not found: {0}")]
    FileNotFound(String),
    
    /// General errors
    #[error("{0}")]
    GeneralError(String),
}

/// Result type for BullRs operations
pub type Result<T> = std::result::Result<T, BullRsError>;
