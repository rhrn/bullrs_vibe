use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::BullRsError;

/// Represents the pattern for a repeatable job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RepeatPattern {
    /// Every n milliseconds
    Every { milliseconds: u64 },
    /// Cron expression (not yet implemented fully)
    Cron { expression: String },
}

impl fmt::Display for RepeatPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepeatPattern::Every { milliseconds } => write!(f, "every:{}", milliseconds),
            RepeatPattern::Cron { expression } => write!(f, "cron:{}", expression),
        }
    }
}

/// Options for a repeatable job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatOptions {
    /// The pattern for repeating the job
    pub pattern: RepeatPattern,
    /// Optional limit to the number of times the job is repeated
    pub limit: Option<u32>,
    /// Optional start time for the first repetition
    pub start_time: Option<u64>,
    /// Optional end time after which the job stops repeating
    pub end_time: Option<u64>,
}

impl Default for RepeatOptions {
    fn default() -> Self {
        RepeatOptions {
            pattern: RepeatPattern::Every { milliseconds: 3600000 }, // default to hourly
            limit: None,
            start_time: None,
            end_time: None,
        }
    }
}

/// Generate a unique key for a repeatable job based on its name and pattern
pub fn get_repeat_key(name: &str, pattern: &RepeatPattern) -> String {
    format!("{}:{}", name, pattern)
}

/// Calculate the next time a job should run based on its pattern and the current time
pub fn get_next_time(pattern: &RepeatPattern, prev_time: Option<u64>) -> Result<u64, BullRsError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| BullRsError::SystemTimeError(e.to_string()))?
        .as_millis() as u64;
    
    match pattern {
        RepeatPattern::Every { milliseconds } => {
            if let Some(prev) = prev_time {
                Ok(prev + milliseconds)
            } else {
                Ok(now + milliseconds)
            }
        },
        RepeatPattern::Cron { expression: _ } => {
            // TODO: Implement cron expression parsing
            // For now, just return a time 24 hours later as a placeholder
            if let Some(prev) = prev_time {
                Ok(prev + 86400000) // 24 hours
            } else {
                Ok(now + 86400000)
            }
        }
    }
}

/// Check if a repeatable job should be removed based on its limit and end time
pub fn should_remove_repeatable(
    options: &RepeatOptions,
    repeat_count: u32,
) -> Result<bool, BullRsError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| BullRsError::SystemTimeError(e.to_string()))?
        .as_millis() as u64;
    
    // Check if limit is reached
    if let Some(limit) = options.limit {
        if repeat_count >= limit {
            return Ok(true);
        }
    }
    
    // Check if end time is reached
    if let Some(end_time) = options.end_time {
        if now >= end_time {
            return Ok(true);
        }
    }
    
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_repeat_pattern_display() {
        let every = RepeatPattern::Every { milliseconds: 5000 };
        assert_eq!(every.to_string(), "every:5000");
        
        let cron = RepeatPattern::Cron { expression: "* * * * *".to_string() };
        assert_eq!(cron.to_string(), "cron:* * * * *");
    }
    
    #[test]
    fn test_get_repeat_key() {
        let pattern = RepeatPattern::Every { milliseconds: 5000 };
        let key = get_repeat_key("test-job", &pattern);
        assert_eq!(key, "test-job:every:5000");
    }
    
    #[test]
    fn test_get_next_time() {
        let pattern = RepeatPattern::Every { milliseconds: 5000 };
        
        // Test with previous time
        let prev_time = 1000000;
        let next_time = get_next_time(&pattern, Some(prev_time)).unwrap();
        assert_eq!(next_time, prev_time + 5000);
        
        // Test without previous time (from now)
        let next_time = get_next_time(&pattern, None).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        // Check that next time is approximately now + 5000 ms
        assert!(next_time >= now + 5000);
        assert!(next_time <= now + 5500); // Allow small buffer for test execution
    }
    
    #[test]
    fn test_should_remove_repeatable() {
        // Test with limit reached
        let options = RepeatOptions {
            pattern: RepeatPattern::Every { milliseconds: 5000 },
            limit: Some(10),
            start_time: None,
            end_time: None,
        };
        
        assert!(!should_remove_repeatable(&options, 9).unwrap());
        assert!(should_remove_repeatable(&options, 10).unwrap());
        
        // Test with end time reached
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let options = RepeatOptions {
            pattern: RepeatPattern::Every { milliseconds: 5000 },
            limit: None,
            start_time: None,
            end_time: Some(now - 1000), // End time in the past
        };
        
        assert!(should_remove_repeatable(&options, 5).unwrap());
        
        // Test with neither limit nor end time reached
        let options = RepeatOptions {
            pattern: RepeatPattern::Every { milliseconds: 5000 },
            limit: Some(20),
            start_time: None,
            end_time: Some(now + 10000), // End time in the future
        };
        
        assert!(!should_remove_repeatable(&options, 5).unwrap());
    }
}
