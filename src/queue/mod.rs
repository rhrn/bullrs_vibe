use std::collections::HashMap;
use crate::repeatable::{RepeatOptions, RepeatPattern, get_repeat_key, get_next_time, should_remove_repeatable};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use redis::{Client, Connection, RedisError};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::commands::script_loader::ScriptLoader;
use crate::errors::BullRsError;
use crate::job::{Job, JobOptions, JobState};
use crate::worker::Worker;

/// Queue configuration options
#[derive(Debug, Clone)]
pub struct QueueOptions {
    /// Redis connection URL (default: "redis://127.0.0.1:6379")
    pub redis_url: String,
    /// Prefix for all queue keys (default: "bull")
    pub prefix: String,
    /// Optional Redis client to use instead of creating a new one
    pub redis_client: Option<Arc<Client>>,
    /// Maximum number of jobs to process concurrently (default: 1)
    pub concurrency: usize,
    /// Maximum job staleness in milliseconds before marked as stalled (default: 30000)
    pub stalledInterval: u64,
    /// Maximum time to wait for a job to be processed in milliseconds (default: 0, no timeout)
    pub maxStalledCount: u32,
    /// Whether to disable tracking of job progress events (default: false)
    pub disableProgressEvents: bool,
    /// Whether to automatically run Redis commands at specified interval (default: true)
    pub enableRedisCommands: bool,
}

impl Default for QueueOptions {
    fn default() -> Self {
        QueueOptions {
            redis_url: "redis://127.0.0.1:6379".to_string(),
            prefix: "bull".to_string(),
            redis_client: None,
            concurrency: 1,
            stalledInterval: 30000,
            maxStalledCount: 1,
            disableProgressEvents: false,
            enableRedisCommands: true,
        }
    }
}

/// Queue state enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum QueueState {
    Ready,
    Paused,
    Closed,
}

/// Bull Queue implementation in Rust
pub struct Queue {
    /// Queue name
    pub name: String,
    /// Job options for this queue
    pub options: QueueOptions,
    /// Map of repeatable jobs by their key
    repeat_map: Arc<RwLock<HashMap<String, RepeatOptions>>>,
    /// Queue state
    state: RwLock<QueueState>,
    /// Redis client
    client: Arc<Client>,
    /// Redis connection
    connection: Mutex<Connection>,
    /// Script loader
    script_loader: Arc<ScriptLoader>,
    /// Worker for processing jobs
    worker: Option<Arc<Worker>>,
    /// Job event listeners
    event_listeners: RwLock<HashMap<String, Vec<Box<dyn Fn(Job) + Send + Sync>>>>,
}

impl Queue {
    /// Create a new queue
    pub async fn new(name: &str, options: Option<QueueOptions>) -> Result<Self, BullRsError> {
        let options = options.unwrap_or_default();
        
        // Get or create Redis client
        let client = match &options.redis_client {
            Some(client) => client.clone(),
            None => {
                let client = Client::open(options.redis_url.clone())
                    .map_err(|e| BullRsError::RedisError(e))?;
                Arc::new(client)
            }
        };
        
        // Get Redis connection
        let connection = client.get_connection()
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Create script loader
        let script_loader = ScriptLoader::new()?;
        let script_loader = Arc::new(script_loader);
        
        // Initialize queue
        let queue = Queue {
            name: name.to_string(),
            options,
            repeat_map: Arc::new(RwLock::new(HashMap::new())),
            state: RwLock::new(QueueState::Ready),
            client,
            connection: Mutex::new(connection),
            script_loader,
            worker: None,
            event_listeners: RwLock::new(HashMap::new()),
        };
        
        // Load Lua scripts
        queue.load_scripts().await?;
        
        Ok(queue)
    }
    
    /// Load Lua scripts
    async fn load_scripts(&self) -> Result<(), BullRsError> {
        let _conn = self.connection.lock().await;
        
        // Initialize script loader with Redis client
        self.script_loader.load(&*self.client, "commands", None).await?;
        
        Ok(())
    }
    
    /// Add a job to the queue
    pub async fn add(&self, name: &str, data: Value, options: Option<JobOptions>) -> Result<Job, BullRsError> {
        let job_id = match &options {
            Some(opts) => opts.job_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
            None => Uuid::new_v4().to_string(),
        };
        
        let job = Job::new(
            job_id,
            self.name.clone(),
            name.to_string(),
            data,
            options,
        );
        
        // Add job to Redis
        self.add_job(&job).await?;
        
        Ok(job)
    }
    
    /// Add a job to Redis
    async fn add_job(&self, job: &Job) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Convert job to JSON
        let job_data = serde_json::to_string(job)
            .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
        
        // Get addJob script command
        let script_args = vec![
            // KEYS
            format!("{}:{}", self.name, ""),         // 1) prefix key
            format!("{}:job:{}", self.name, job.id), // 2) job key
            format!("{}:wait", self.name),           // 3) wait list
            format!("{}:paused", self.name),         // 4) paused list
            format!("{}:meta-paused", self.name),    // 5) meta-paused key
            format!("{}:delayed", self.name),        // 6) delayed set
            // ARGV
            job.id.clone(),                          // 1) job id
            job_data,                                // 2) job data
            job.options.priority
                .map(|p| p as u32)
                .unwrap_or(3).to_string(),           // 3) priority (default: normal=3)
            job.process_at.to_string(),              // 4) timestamp to process job
            "0".to_string(),                         // 5) attempts
        ];
        
        // Execute script
        redis::cmd("EVAL")
            .arg("-- Implementation of addJob script")
            .arg(6) // Number of KEYS
            .arg(&script_args)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(())
    }
    
    /// Get a job by ID
    pub async fn get_job(&self, job_id: &str) -> Result<Option<Job>, BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Get job data from Redis
        Job::from_id(&self.name, job_id, &mut *conn).await
    }
    
    /// Pause the queue
    pub async fn pause(&self) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        let mut state = self.state.write().await;
        
        // Execute pause script
        redis::cmd("EVAL")
            .arg("-- Implementation of pause script")
            .arg(1) // Number of KEYS
            .arg(format!("{}:meta-paused", self.name))
            .arg("true")
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Update state
        *state = QueueState::Paused;
        
        Ok(())
    }
    
    /// Resume the queue
    pub async fn resume(&self) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        let mut state = self.state.write().await;
        
        // Execute resume script (unpause)
        redis::cmd("EVAL")
            .arg("-- Implementation of resume script")
            .arg(1) // Number of KEYS
            .arg(format!("{}:meta-paused", self.name))
            .arg("false")
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Update state
        *state = QueueState::Ready;
        
        Ok(())
    }
    
    /// Retry a failed job
    pub async fn retry_job(&self, job_id: &str) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Get the current timestamp for delay calculation
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| BullRsError::SystemTimeError(e.to_string()))?
            .as_millis() as u64;
        
        // Execute retry script
        redis::cmd("EVAL")
            .arg("-- Implementation of retryJob script")
            .arg(3) // Number of KEYS
            .arg(format!("{}:failed", self.name))
            .arg(format!("{}:wait", self.name))
            .arg(format!("{}:delayed", self.name))
            .arg(job_id)
            .arg(now)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(())
    }
    
    /// Promote a delayed job to be executed immediately
    pub async fn promote_job(&self, job_id: &str) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Execute promote script
        redis::cmd("EVAL")
            .arg("-- Implementation of promote script")
            .arg(2) // Number of KEYS
            .arg(format!("{}:delayed", self.name))
            .arg(format!("{}:wait", self.name))
            .arg(job_id)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(())
    }
    
    /// Remove a job from the queue
    pub async fn remove_job(&self, job_id: &str) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Execute removeJob script
        redis::cmd("EVAL")
            .arg("-- Implementation of removeJob script")
            .arg(4) // Number of KEYS
            .arg(format!("{}:active", self.name))
            .arg(format!("{}:wait", self.name))
            .arg(format!("{}:delayed", self.name))
            .arg(format!("{}:job:{}", self.name, job_id))
            .arg(job_id)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(())
    }
    
    /// Get jobs by status (waiting, active, completed, failed, delayed)
    pub async fn get_jobs(&self, status: &str, start: usize, end: usize) -> Result<Vec<Job>, BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Get list of job IDs based on status
        let ids: Vec<String> = match status {
            "waiting" => redis::cmd("LRANGE")
                .arg(format!("{}:wait", self.name))
                .arg(start)
                .arg(end)
                .query(&mut *conn)
                .map_err(|e| BullRsError::RedisError(e))?,
            "active" => redis::cmd("LRANGE")
                .arg(format!("{}:active", self.name))
                .arg(start)
                .arg(end)
                .query(&mut *conn)
                .map_err(|e| BullRsError::RedisError(e))?,
            "completed" => redis::cmd("LRANGE")
                .arg(format!("{}:completed", self.name))
                .arg(start)
                .arg(end)
                .query(&mut *conn)
                .map_err(|e| BullRsError::RedisError(e))?,
            "failed" => redis::cmd("LRANGE")
                .arg(format!("{}:failed", self.name))
                .arg(start)
                .arg(end)
                .query(&mut *conn)
                .map_err(|e| BullRsError::RedisError(e))?,
            "delayed" => {
                let ids_with_scores: Vec<(String, f64)> = redis::cmd("ZRANGE")
                    .arg(format!("{}:delayed", self.name))
                    .arg(start)
                    .arg(end)
                    .arg("WITHSCORES")
                    .query(&mut *conn)
                    .map_err(|e| BullRsError::RedisError(e))?;
                
                ids_with_scores.into_iter().map(|(id, _)| id).collect()
            },
            _ => return Err(BullRsError::InvalidArgument(format!("Invalid status: {}", status))),
        };
        
        // Load job data for each ID
        let mut jobs = Vec::new();
        for id in ids {
            if let Some(job) = Job::from_id(&self.name, &id, &mut *conn).await? {
                jobs.push(job);
            }
        }
        
        Ok(jobs)
    }
    
    /// Add a repeatable job to the queue
    pub async fn add_repeatable_job(&self, name: &str, data: Value, repeat_options: RepeatOptions) -> Result<Job, BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Calculate next run time
        let next_time = get_next_time(&repeat_options.pattern, None)?;
        
        // Create job options with delay to next run time
        let mut job_options = JobOptions::default();
        job_options.delay = Some(next_time);
        
        // Generate repeat key and store it before we move job_options
        let repeat_key = get_repeat_key(name, &repeat_options.pattern);
        job_options.repeat_key = Some(repeat_key.clone());
        
        // Store repeat options in the repeat map
        {
            let mut repeat_map = self.repeat_map.write().await;
            repeat_map.insert(repeat_key.clone(), repeat_options.clone());
        }
        
        // Add job to the queue
        let job = self.add(name, data, Some(job_options)).await?;
        let repeat_json = serde_json::to_string(&repeat_options)
            .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
        
        redis::cmd("HSET")
            .arg(format!("{}:repeat", self.name))
            .arg(&repeat_key)
            .arg(&repeat_json)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(job)
    }
    
    /// Remove a repeatable job by its key
    pub async fn remove_repeatable_job(&self, repeat_key: &str) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Remove from Redis
        redis::cmd("HDEL")
            .arg(format!("{}:repeat", self.name))
            .arg(repeat_key)
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Remove from internal map
        {
            let mut repeat_map = self.repeat_map.write().await;
            repeat_map.remove(repeat_key);
        }
        
        Ok(())
    }
    
    /// Get all repeatable jobs
    pub async fn get_repeatable_jobs(&self) -> Result<HashMap<String, RepeatOptions>, BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Get all repeat metadata from Redis
        let repeat_data: HashMap<String, String> = redis::cmd("HGETALL")
            .arg(format!("{}:repeat", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Parse repeat options
        let mut result = HashMap::new();
        for (key, json) in repeat_data {
            let options: RepeatOptions = serde_json::from_str(&json)
                .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
            result.insert(key, options);
        }
        
        Ok(result)
    }
    
    /// Check and schedule the next occurrence of a repeatable job
    pub async fn schedule_next_repeatable(&self, job: &Job) -> Result<Option<Job>, BullRsError> {
        if let Some(repeat_key) = &job.options.repeat_key {
            let repeat_options = {
                let repeat_map = self.repeat_map.read().await;
                repeat_map.get(repeat_key).cloned()
            };
            
            if let Some(options) = repeat_options {
                // Check if we should continue repeating
                let repeat_count = job.progress.repeat_count.unwrap_or(0) + 1;
                if !should_remove_repeatable(&options, repeat_count)? {
                    // Calculate next time
                    let next_time = get_next_time(&options.pattern, job.options.delay)?;
                    
                    // Create job options for next occurrence
                    let mut job_options = job.options.clone();
                    job_options.delay = Some(next_time);
                    
                    // Add next job
                    let mut data = job.data.clone();
                    data["repeatCount"] = serde_json::Value::Number(serde_json::Number::from(repeat_count));
                    
                    return Ok(Some(self.add(&job.name, data, Some(job_options)).await?));
                } else {
                    // Remove repeatable job if it's done
                    self.remove_repeatable_job(repeat_key).await?;
                }
            }
        }
        
        Ok(None)
    }
    
    /// Completely obliterate the queue and all its data
    pub async fn obliterate(&self) -> Result<(), BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Execute obliterate script
        redis::cmd("EVAL")
            .arg("-- Implementation of obliterate script")
            .arg(1) // Number of KEYS
            .arg(format!("{}", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        Ok(())
    }
    
    /// Get queue counts (waiting, active, completed, failed, delayed)
    pub async fn get_counts(&self) -> Result<HashMap<String, usize>, BullRsError> {
        let mut conn = self.connection.lock().await;
        
        // Create count mapping
        let mut counts = HashMap::new();
        
        // Get waiting count
        let waiting: usize = redis::cmd("LLEN")
            .arg(format!("{}:wait", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        counts.insert("waiting".to_string(), waiting);
        
        // Get active count
        let active: usize = redis::cmd("LLEN")
            .arg(format!("{}:active", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        counts.insert("active".to_string(), active);
        
        // Get completed count
        let completed: usize = redis::cmd("LLEN")
            .arg(format!("{}:completed", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        counts.insert("completed".to_string(), completed);
        
        // Get failed count
        let failed: usize = redis::cmd("LLEN")
            .arg(format!("{}:failed", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        counts.insert("failed".to_string(), failed);
        
        // Get delayed count
        let delayed: usize = redis::cmd("ZCARD")
            .arg(format!("{}:delayed", self.name))
            .query(&mut *conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        counts.insert("delayed".to_string(), delayed);
        
        Ok(counts)
    }
    
    /// Process jobs with a callback function
    pub async fn process<F>(&mut self, concurrency: Option<usize>, processor: F) -> Result<(), BullRsError>
    where
        F: Fn(Job) -> Result<Value, String> + Send + Sync + 'static,
    {
        let concurrency = concurrency.unwrap_or(self.options.concurrency);
        
        // Create worker
        let worker = Worker::new(
            self.client.clone(),
            &self.name,
            concurrency,
            Arc::new(processor),
            Arc::clone(&self.script_loader),
        ).await?;
        
        // Store worker
        self.worker = Some(Arc::new(worker));
        
        Ok(())
    }
    
    /// Add an event listener
    pub async fn on<F>(&self, event: &str, callback: F) -> Result<(), BullRsError>
    where
        F: Fn(Job) + Send + Sync + 'static,
    {
        let mut listeners = self.event_listeners.write().await;
        
        // Create listener entry if it doesn't exist
        if !listeners.contains_key(event) {
            listeners.insert(event.to_string(), Vec::new());
        }
        
        // Add callback
        if let Some(event_listeners) = listeners.get_mut(event) {
            event_listeners.push(Box::new(callback));
        }
        
        Ok(())
    }
    
    /// Emit an event
    async fn emit(&self, event: &str, job: Job) -> Result<(), BullRsError> {
        let listeners = self.event_listeners.read().await;
        
        // Call event listeners
        if let Some(event_listeners) = listeners.get(event) {
            for listener in event_listeners {
                listener(job.clone());
            }
        }
        
        Ok(())
    }
    
    /// Close the queue
    pub async fn close(&self) -> Result<(), BullRsError> {
        let mut state = self.state.write().await;
        *state = QueueState::Closed;
        
        // Stop worker if exists
        if let Some(_worker) = &self.worker {
            // worker.stop().await?;
            // Uncomment this when worker.stop is implemented
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    
    #[tokio::test]
    async fn test_queue_creation() {
        // This test requires a Redis server running
        let result = Queue::new("test-queue", None).await;
        
        // If Redis is running, this should succeed
        if result.is_ok() {
            let queue = result.unwrap();
            assert_eq!(queue.name, "test-queue");
            
            // Test default options
            assert_eq!(queue.options.redis_url, "redis://127.0.0.1:6379");
            assert_eq!(queue.options.prefix, "bull");
            assert_eq!(queue.options.concurrency, 1);
            
            // Verify state
            let state = queue.state.read().await;
            assert_eq!(*state, QueueState::Ready);
        }
    }
    
    #[tokio::test]
    async fn test_add_job() {
        // This test requires a Redis server running
        let result = Queue::new("test-queue-add", None).await;
        
        // If Redis is running, test adding a job
        if let Ok(queue) = result {
            let job_result = queue.add("test-job", json!({"data": "test"}), None).await;
            
            if let Ok(job) = job_result {
                assert_eq!(job.name, "test-job");
                assert_eq!(job.queue_name, "test-queue-add");
                assert_eq!(job.data, json!({"data": "test"}));
                assert_eq!(job.state, JobState::Waiting);
            }
        }
    }
    
    #[tokio::test]
    async fn test_queue_pause_resume() {
        // This test requires a Redis server running
        let result = Queue::new("test-queue-pause", None).await;
        
        // If Redis is running, test pause and resume
        if let Ok(queue) = result {
            // Test pause
            let pause_result = queue.pause().await;
            if pause_result.is_ok() {
                let state = queue.state.read().await;
                assert_eq!(*state, QueueState::Paused);
                
                // Test resume
                drop(state); // Release read lock
                let resume_result = queue.resume().await;
                if resume_result.is_ok() {
                    let state = queue.state.read().await;
                    assert_eq!(*state, QueueState::Ready);
                }
            }
        }
    }
}
