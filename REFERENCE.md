# BullRs Reference Documentation

BullRs is a Rust implementation of the popular Bull queue library for Node.js. It provides a Redis-backed job queueing system for Rust applications.

## Table of Contents

- [Queue](#queue)
  - [Queue Constructor](#queue-constructor)
  - [Queue Properties](#queue-properties)
- [Jobs](#jobs)
  - [Job Properties](#job-properties)
  - [Job Options](#job-options)
- [Events](#events)
- [Methods and Functions](#methods-and-functions)
  - [Queue Methods](#queue-methods)
  - [Job Methods](#job-methods)
- [Repeatable Jobs](#repeatable-jobs)
- [Worker](#worker)
- [Error Handling](#error-handling)

## Queue

A Queue is the main class in BullRs. It represents a queue of jobs stored in Redis.

### Queue Constructor

```rust
pub async fn new(
    name: &str,
    client: Arc<Client>,
    options: QueueOptions,
) -> Result<Self, BullRsError> {
    // ...
}
```

Parameters:
- `name`: The name of the queue
- `client`: A Redis client
- `options`: Queue options

### Queue Properties

```rust
pub struct Queue {
    /// Queue name
    pub name: String,
    /// Job options for this queue
    pub options: QueueOptions,
    /// Queue state
    state: RwLock<QueueState>,
    /// Redis client
    client: Arc<Client>,
    /// Redis connection
    connection: Mutex<Connection>,
    /// Map of repeatable jobs by their key
    repeat_map: Arc<RwLock<HashMap<String, RepeatOptions>>>,
}
```

### Queue Options

```rust
pub struct QueueOptions {
    /// Redis prefix for all queue keys
    pub prefix: String,
    /// Default job options
    pub default_job_options: JobOptions,
    /// Check for stalled jobs interval in milliseconds
    pub stalledInterval: u64,
    /// Max number of times a job can be marked as stalled
    pub maxStalledCount: u32,
    /// Disable progress event emissions
    pub disableProgressEvents: bool,
    /// Enable Redis commands
    pub enableRedisCommands: bool,
}
```

## Jobs

A Job represents a task to be processed by a worker.

### Job Properties

```rust
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
```

### Job Options

```rust
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
```

### Job Progress

```rust
pub struct JobProgress {
    /// Progress percentage (0-100)
    pub percentage: f64,
    /// Optional data associated with the progress update
    pub data: Option<Value>,
    /// Number of times the job has been repeated (for repeatable jobs)
    pub repeat_count: Option<u32>,
}
```

## Events

BullRs supports the following events:

- Job added
- Job completed
- Job failed
- Job progress updated
- Job stalled
- Job removed
- Queue paused
- Queue resumed

## Methods and Functions

### Queue Methods

#### Add a Job

```rust
pub async fn add(
    &self, 
    name: &str, 
    data: Value, 
    options: Option<JobOptions>
) -> Result<Job, BullRsError>
```

Adds a job to the queue.

#### Pause Queue

```rust
pub async fn pause(&self) -> Result<(), BullRsError>
```

Pauses the queue, preventing jobs from being processed.

#### Resume Queue

```rust
pub async fn resume(&self) -> Result<(), BullRsError>
```

Resumes a paused queue.

#### Get Job Counts

```rust
pub async fn get_counts(&self) -> Result<HashMap<String, usize>, BullRsError>
```

Gets the counts of jobs in different states.

#### Get Jobs by Status

```rust
pub async fn get_jobs(
    &self, 
    status: &str, 
    start: usize, 
    end: usize
) -> Result<Vec<Job>, BullRsError>
```

Gets jobs by status with pagination.

#### Clean Queue

```rust
pub async fn clean(
    &self, 
    grace_period: u64, 
    status: &str, 
    limit: usize
) -> Result<usize, BullRsError>
```

Cleans the queue by removing jobs of a certain status.

#### Add Repeatable Job

```rust
pub async fn add_repeatable_job(
    &self, 
    name: &str, 
    data: Value, 
    repeat_options: RepeatOptions
) -> Result<Job, BullRsError>
```

Adds a repeatable job to the queue.

#### Remove Repeatable Job

```rust
pub async fn remove_repeatable_job(
    &self, 
    repeat_key: &str
) -> Result<(), BullRsError>
```

Removes a repeatable job by its key.

#### Get Repeatable Jobs

```rust
pub async fn get_repeatable_jobs(
    &self
) -> Result<HashMap<String, RepeatOptions>, BullRsError>
```

Gets all repeatable jobs.

#### Remove Job

```rust
pub async fn remove_job(
    &self, 
    job_id: &str
) -> Result<(), BullRsError>
```

Removes a job from the queue.

#### Retry Job

```rust
pub async fn retry_job(
    &self, 
    job_id: &str
) -> Result<(), BullRsError>
```

Retries a failed job.

#### Promote Job

```rust
pub async fn promote_job(
    &self, 
    job_id: &str
) -> Result<(), BullRsError>
```

Promotes a delayed job to be executed immediately.

#### Obliterate Queue

```rust
pub async fn obliterate(&self) -> Result<(), BullRsError>
```

Completely removes the queue and all its data.

### Job Methods

#### Update Progress

```rust
pub fn update_progress(&mut self, percentage: f64) -> Result<(), BullRsError>
```

Updates the progress of the job.

#### Update Progress with Data

```rust
pub fn update_progress_with_data(
    &mut self, 
    percentage: f64, 
    data: Value
) -> Result<(), BullRsError>
```

Updates the progress of the job with additional data.

#### Mark as Active

```rust
pub fn mark_as_active(&mut self)
```

Marks the job as active.

#### Mark as Completed

```rust
pub fn mark_as_completed(&mut self, return_value: Option<Value>)
```

Marks the job as completed.

#### Mark as Failed

```rust
pub fn mark_as_failed(&mut self, err: &str, stack_trace: Option<String>)
```

Marks the job as failed.

#### Can Retry

```rust
pub fn can_retry(&self) -> bool
```

Checks if the job can be retried.

## Repeatable Jobs

BullRs supports repeatable jobs, which are automatically re-added to the queue at specified intervals.

### Repeat Pattern

```rust
pub enum RepeatPattern {
    /// Every n milliseconds
    Every { milliseconds: u64 },
    /// Cron expression
    Cron { expression: String },
}
```

### Repeat Options

```rust
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
```

## Worker

A Worker is responsible for processing jobs from a queue.

```rust
pub struct Worker {
    /// Redis client
    client: Arc<Client>,
    /// Queue name
    queue_name: String,
    /// Concurrency level
    concurrency: usize,
    /// Job processor function
    processor: JobProcessor,
    /// Script loader for Redis commands
    script_loader: Arc<ScriptLoader>,
    /// Worker state
    running: RwLock<bool>,
    /// Worker stop channel
    stop_channel: (mpsc::Sender<()>, Mutex<mpsc::Receiver<()>>),
    /// Active job map
    active_jobs: Arc<RwLock<HashMap<String, Job>>>,
}
```

### Worker Methods

#### Creating a New Worker

```rust
pub async fn new(
    client: Arc<Client>,
    queue_name: &str,
    concurrency: usize,
    processor: JobProcessor,
    script_loader: Arc<ScriptLoader>,
) -> Result<Self, BullRsError>
```

Creates a new worker.

#### Stop Worker

```rust
pub async fn stop(&self) -> Result<(), BullRsError>
```

Stops the worker.

#### Get Active Job Count

```rust
pub async fn active_job_count(&self) -> usize
```

Gets the number of active jobs.

## Error Handling

BullRs uses a custom error type `BullRsError` for error handling:

```rust
pub enum BullRsError {
    /// Error related to Redis operations
    RedisError(redis::RedisError),
    /// Error related to IO operations
    IoError(std::io::Error),
    /// Error related to script loading or execution
    ScriptError(String),
    /// Error when parsing lua scripts
    ParsingError(String),
    /// Error for circular references
    CircularReferenceError(String),
    /// Error when a path mapping is not found
    PathMappingNotFoundError(String),
    /// Error related to serialization or deserialization
    SerializationError(String),
    /// Error related to process execution
    ProcessError(String),
    /// Error when a process times out
    ProcessTimeout,
    /// Error when maximum process limit is reached
    ProcessLimitReached,
    /// Error related to system time operations
    SystemTimeError(String),
    /// Error when an invalid argument is provided
    InvalidArgument(String),
    /// Error when a file is not found
    FileNotFound(String),
    /// General errors
    GeneralError(String),
}
```

## Usage Examples

### Basic Queue Usage

```rust
use bullrs::{Queue, QueueOptions, Job};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue_options = QueueOptions::default();
    let queue = Queue::new("my-queue", client, queue_options).await?;
    
    // Add a job
    let job = queue.add(
        "test-job",
        json!({"foo": "bar"}),
        None,
    ).await?;
    
    println!("Added job with ID: {}", job.id);
    
    Ok(())
}
```

### Creating a Worker

```rust
use bullrs::{Queue, QueueOptions, Worker, Job, ScriptLoader};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Initialize script loader
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Define a job processor
    let processor = Arc::new(|job: Job| -> Result<serde_json::Value, String> {
        println!("Processing job {}: {:?}", job.id, job.data);
        // Process the job...
        Ok(json!({"result": "success"}))
    });
    
    // Create worker
    let worker = Worker::new(
        client.clone(),
        "my-queue",
        5, // concurrency
        processor,
        script_loader,
    ).await?;
    
    // Wait for jobs...
    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    
    // Stop worker
    worker.stop().await?;
    
    Ok(())
}
```

### Adding a Repeatable Job

```rust
use bullrs::{Queue, QueueOptions, RepeatOptions, RepeatPattern};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue_options = QueueOptions::default();
    let queue = Queue::new("my-queue", client, queue_options).await?;
    
    // Define repeat options (every 10 seconds)
    let repeat_options = RepeatOptions {
        pattern: RepeatPattern::Every { milliseconds: 10000 },
        limit: Some(5), // Repeat 5 times
        start_time: None,
        end_time: None,
    };
    
    // Add a repeatable job
    let job = queue.add_repeatable_job(
        "repeat-job",
        json!({"task": "repeating task"}),
        repeat_options,
    ).await?;
    
    println!("Added repeatable job with ID: {}", job.id);
    
    Ok(())
}
```
