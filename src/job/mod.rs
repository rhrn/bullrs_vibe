use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use redis::{Connection, RedisError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::BullRsError;
use crate::commands::script_loader::CommandDefinition;

/// Job state enumeration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JobState {
    Waiting,
    Active,
    Completed,
    Failed,
    Delayed,
    Paused,
    Stalled,
}

/// Job priority levels
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JobPriority {
    Critical = 1,
    High = 2,
    Normal = 3,
    Low = 4,
    Lowest = 5,
}

impl Default for JobPriority {
    fn default() -> Self {
        JobPriority::Normal
    }
}

/// Job options for creating a new job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOptions {
    /// Optional job priority
    pub priority: Option<JobPriority>,
    /// Optional delay in milliseconds
    pub delay: Option<u64>,
    /// Optional attempts before job fails
    pub attempts: Option<u32>,
    /// Optional backoff strategy
    pub backoff: Option<Backoff>,
    /// Optional time to live in milliseconds
    pub ttl: Option<u64>,
    /// Optional remove on complete
    pub remove_on_complete: Option<bool>,
    /// Optional remove on fail
    pub remove_on_fail: Option<bool>,
    /// Optional job ID to use
    pub job_id: Option<String>,
    /// Optional timestamp to use
    pub timestamp: Option<u64>,
    /// Optional repeat key for repeatable jobs
    pub repeat_key: Option<String>,
}

impl Default for JobOptions {
    fn default() -> Self {
        JobOptions {
            priority: None,
            delay: None,
            attempts: None,
            backoff: None,
            ttl: None,
            remove_on_complete: None,
            remove_on_fail: None,
            job_id: None,
            timestamp: None,
            repeat_key: None,
        }
    }
}

/// Backoff strategy for retrying failed jobs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backoff {
    /// Type of backoff strategy (e.g., "fixed", "exponential")
    pub strategy_type: String,
    /// Delay in milliseconds for the backoff
    pub delay: u64,
}

/// Job progress tracking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobProgress {
    /// Progress percentage (0-100)
    pub percentage: f64,
    /// Optional data associated with the progress update
    pub data: Option<Value>,
    /// Number of times the job has been repeated (for repeatable jobs)
    pub repeat_count: Option<u32>,
}

impl Default for JobProgress {
    fn default() -> Self {
        JobProgress {
            percentage: 0.0,
            data: None,
            repeat_count: None,
        }
    }
}

/// Job structure representing a task in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique job ID
    pub id: String,
    /// Queue name this job belongs to
    pub queue_name: String,
    /// Job name
    pub name: String,
    /// Job data
    pub data: Value,
    /// Job options
    pub options: JobOptions,
    /// Job progress tracking
    pub progress: JobProgress,
    /// Current state of the job
    pub state: JobState,
    /// Number of attempts performed
    pub attempts_made: u32,
    /// Maximum number of attempts
    pub max_attempts: u32,
    /// Optional stack trace if the job failed
    pub stack_trace: Option<String>,
    /// Optional result data if the job completed
    pub return_value: Option<Value>,
    /// Job creation timestamp in milliseconds since epoch
    pub created_at: u64,
    /// Job process start timestamp in milliseconds since epoch
    pub started_at: Option<u64>,
    /// Job finish timestamp in milliseconds since epoch
    pub finished_at: Option<u64>,
    /// Job process timestamp in milliseconds since epoch
    pub process_at: u64,
    /// Optional promotion timestamp in milliseconds since epoch
    pub promoted_at: Option<u64>,
}

impl Job {
    /// Create a new job
    pub fn new(
        id: String,
        queue_name: String,
        name: String,
        data: Value,
        opts: Option<JobOptions>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let opts = opts.unwrap_or_default();
        let max_attempts = opts.attempts.unwrap_or(1);
        let delay = opts.delay.unwrap_or(0);
        let process_at = if delay > 0 { now + delay } else { now };
        
        Job {
            id,
            queue_name,
            name,
            data,
            options: opts,
            progress: JobProgress::default(),
            state: JobState::Waiting,
            attempts_made: 0,
            max_attempts,
            stack_trace: None,
            return_value: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            process_at,
            promoted_at: None,
        }
    }
    
    /// Update job progress
    pub fn update_progress(&mut self, percentage: f64) -> Result<(), BullRsError> {
        // Validate progress is between 0 and 100
        if percentage < 0.0 || percentage > 100.0 {
            return Err(BullRsError::InvalidArgument(format!("Progress must be between 0 and 100, got {}", percentage)));
        }
        
        self.progress.percentage = percentage;
        Ok(())
    }
    
    /// Update job progress with additional data
    pub fn update_progress_with_data(&mut self, percentage: f64, data: Value) -> Result<(), BullRsError> {
        self.update_progress(percentage)?;
        self.progress.data = Some(data);
        Ok(())
    }
    
    /// Mark the job as active
    pub fn mark_as_active(&mut self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        self.state = JobState::Active;
        self.started_at = Some(now);
        self.attempts_made += 1;
    }
    
    /// Mark the job as completed
    pub fn mark_as_completed(&mut self, return_value: Option<Value>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        self.state = JobState::Completed;
        self.return_value = return_value;
        self.progress.percentage = 100.0;
        self.finished_at = Some(now);
    }
    
    /// Mark the job as failed
    pub fn mark_as_failed(&mut self, err: &str, stack_trace: Option<String>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        self.state = JobState::Failed;
        self.stack_trace = Some(stack_trace.unwrap_or_else(|| err.to_string()));
        self.finished_at = Some(now);
    }
    
    /// Check if the job can be retried
    pub fn can_retry(&self) -> bool {
        self.state == JobState::Failed && self.attempts_made < self.max_attempts
    }
    
    /// Convert the job to a JSON value
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
    
    /// Load a job from Redis
    pub async fn from_id(
        queue_name: &str,
        id: &str,
        conn: &mut Connection,
    ) -> Result<Option<Self>, BullRsError> {
        let key = format!("{}:{}", queue_name, id);
        let job_data: Option<String> = redis::cmd("GET")
            .arg(key)
            .query(conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        match job_data {
            Some(data) => {
                let job: Job = serde_json::from_str(&data)
                    .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
                Ok(Some(job))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    
    #[test]
    fn test_job_creation() {
        let job = Job::new(
            "test_id".to_string(),
            "test_queue".to_string(),
            "test_job".to_string(),
            json!({"foo": "bar"}),
            None,
        );
        
        assert_eq!(job.id, "test_id");
        assert_eq!(job.queue_name, "test_queue");
        assert_eq!(job.name, "test_job");
        assert_eq!(job.data, json!({"foo": "bar"}));
        assert_eq!(job.state, JobState::Waiting);
        assert_eq!(job.attempts_made, 0);
        assert_eq!(job.max_attempts, 1);
        assert_eq!(job.progress.percentage, 0.0);
    }
    
    #[test]
    fn test_job_state_transitions() {
        let mut job = Job::new(
            "test_id".to_string(),
            "test_queue".to_string(),
            "test_job".to_string(),
            json!({"foo": "bar"}),
            None,
        );
        
        // Test marking as active
        job.mark_as_active();
        assert_eq!(job.state, JobState::Active);
        assert!(job.started_at.is_some());
        assert_eq!(job.attempts_made, 1);
        
        // Test marking as completed
        job.mark_as_completed(Some(json!("result")));
        assert_eq!(job.state, JobState::Completed);
        assert!(job.finished_at.is_some());
        assert_eq!(job.progress.percentage, 100.0);
        assert_eq!(job.return_value, Some(json!("result")));
        
        // Create a new job for failure test
        let mut job = Job::new(
            "test_id".to_string(),
            "test_queue".to_string(),
            "test_job".to_string(),
            json!({"foo": "bar"}),
            Some(JobOptions {
                attempts: Some(3),
                ..Default::default()
            }),
        );
        
        // Test marking as failed
        job.mark_as_active();
        job.mark_as_failed("Test error", None);
        assert_eq!(job.state, JobState::Failed);
        assert!(job.finished_at.is_some());
        assert_eq!(job.stack_trace, Some("Test error".to_string()));
        assert!(job.can_retry());
        
        // Test retry exhaustion
        job.mark_as_active();
        job.mark_as_failed("Test error", None);
        job.mark_as_active();
        job.mark_as_failed("Test error", None);
        assert!(!job.can_retry());
    }
    
    #[test]
    fn test_progress_update() {
        let mut job = Job::new(
            "test_id".to_string(),
            "test_queue".to_string(),
            "test_job".to_string(),
            json!({"foo": "bar"}),
            None,
        );
        
        // Test progress is initialized to 0
        assert_eq!(job.progress.percentage, 0.0);
        assert_eq!(job.progress.data, None);
        assert_eq!(job.progress.repeat_count, None);
        
        // Test valid progress update
        assert!(job.update_progress(50.0).is_ok());
        assert_eq!(job.progress.percentage, 50.0);
        
        // Test progress with data update
        assert!(job.update_progress_with_data(75.0, json!({"stage": "processing"})).is_ok());
        assert_eq!(job.progress.percentage, 75.0);
        assert_eq!(job.progress.data, Some(json!({"stage": "processing"})));
        
        // Test invalid progress update
        assert!(job.update_progress(-10.0).is_err());
        assert!(job.update_progress(110.0).is_err());
    }
}
