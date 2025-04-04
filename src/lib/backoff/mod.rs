use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::errors::BullRsError;

/// Backoff strategy types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BackoffStrategy {
    Fixed,
    Exponential,
    Custom(String),
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        BackoffStrategy::Fixed
    }
}

/// Backoff options for job retries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackoffOptions {
    /// Type of backoff strategy
    pub strategy: BackoffStrategy,
    /// Delay in milliseconds
    pub delay: u64,
}

impl Default for BackoffOptions {
    fn default() -> Self {
        BackoffOptions {
            strategy: BackoffStrategy::default(),
            delay: 1000, // Default 1 second delay
        }
    }
}

/// Calculate backoff delay for a job retry
pub fn calculate_backoff(
    options: &BackoffOptions,
    attempts: u32,
) -> u64 {
    match options.strategy {
        BackoffStrategy::Fixed => options.delay,
        BackoffStrategy::Exponential => {
            let base_delay = options.delay;
            let mut delay = base_delay;
            
            // Calculate exponential backoff: delay * 2^(attempts-1)
            for _ in 1..attempts {
                delay = delay.saturating_mul(2);
            }
            
            delay
        }
        BackoffStrategy::Custom(ref _name) => {
            // For custom strategies, we'll use fixed delay for now
            // In a real implementation, this would call a custom function
            options.delay
        }
    }
}

/// Maximum supported backoff delay (1 hour)
pub const MAX_BACKOFF_DELAY: u64 = 3600000;

/// Get a timestamp for when a job should be processed after backoff
pub fn get_backoff_timestamp(
    options: &BackoffOptions,
    attempts: u32,
) -> Result<u64, BullRsError> {
    // Calculate delay
    let delay = calculate_backoff(options, attempts);
    
    // Cap at maximum delay
    let capped_delay = std::cmp::min(delay, MAX_BACKOFF_DELAY);
    
    // Get current timestamp
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| BullRsError::SystemTimeError(e.to_string()))?
        .as_millis() as u64;
    
    // Calculate process timestamp
    Ok(now + capped_delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_fixed_backoff() {
        let options = BackoffOptions {
            strategy: BackoffStrategy::Fixed,
            delay: 5000,
        };
        
        assert_eq!(calculate_backoff(&options, 1), 5000);
        assert_eq!(calculate_backoff(&options, 2), 5000);
        assert_eq!(calculate_backoff(&options, 3), 5000);
    }
    
    #[test]
    fn test_exponential_backoff() {
        let options = BackoffOptions {
            strategy: BackoffStrategy::Exponential,
            delay: 1000,
        };
        
        assert_eq!(calculate_backoff(&options, 1), 1000);
        assert_eq!(calculate_backoff(&options, 2), 2000);
        assert_eq!(calculate_backoff(&options, 3), 4000);
        assert_eq!(calculate_backoff(&options, 4), 8000);
    }
    
    #[test]
    fn test_backoff_timestamp() {
        let options = BackoffOptions {
            strategy: BackoffStrategy::Fixed,
            delay: 5000,
        };
        
        let timestamp = get_backoff_timestamp(&options, 1).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        // Timestamp should be roughly 5 seconds in the future
        assert!(timestamp > now);
        assert!(timestamp <= now + 5100); // Allow small margin for test execution time
    }
}
