# BullRs

[![Crates.io](https://img.shields.io/crates/v/bullrs.svg)](https://crates.io/crates/bullrs)
[![Documentation](https://docs.rs/bullrs/badge.svg)](https://docs.rs/bullrs)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Premium Queue package for handling distributed jobs and messages in Rust.

## Features

- **Minimal CPU usage** due to a polling-free design
- **Robust design** based on Redis
- **Delayed jobs**
- **Repeatable jobs** with fixed intervals or cron expressions
- **Priority queues**
- **Concurrency control**
- **Retries** with configurable backoff strategies
- **Progress tracking** for long-running jobs
- **Pause/resume** - globally or locally
- **Multiple job types** per queue
- **Automatic recovery** from process crashes

## Installation

Add BullRs to your `Cargo.toml`:

```toml
[dependencies]
bullrs = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
redis = "0.23.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

Requirements: BullRs requires a Redis server version greater than or equal to 5.0.

## Quick Guide

```rust
use bullrs::{Queue, QueueOptions, Worker, ScriptLoader};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create a queue with default options
    let queue = Queue::new(
        "my-queue",
        client.clone(),
        QueueOptions::default()
    ).await?;
    
    // Add a job to the queue
    let job = queue.add(
        "my-job",
        json!({ "foo": "bar" }),
        None,
    ).await?;
    
    println!("Added job with ID: {}", job.id);
    
    // Set up a worker to process jobs
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    let processor = Arc::new(|job| {
        println!("Processing job: {:?}", job);
        
        // Process job and return result
        Ok(json!({ "result": "success" }))
    });
    
    let worker = Worker::new(
        client.clone(),
        "my-queue",
        1, // concurrency
        processor,
        script_loader,
    ).await?;
    
    // Let worker process jobs...
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    
    // Gracefully shut down worker
    worker.stop().await?;
    
    Ok(())
}
```

## Documentation

For more information on how to use BullRs, check out these documentation resources:

- [REFERENCE.md](./REFERENCE.md) - Complete API reference
- [PATTERNS.md](./PATTERNS.md) - Common patterns and recipes
- [Examples](./examples) - Example applications

## Examples

BullRs comes with several examples demonstrating common use cases:

- [Basic Queue](./examples/basic_queue.rs) - Simple producer-consumer pattern
- [Repeatable Jobs](./examples/repeatable_jobs.rs) - Creating repeating jobs
- [Error Handling](./examples/error_handling.rs) - Job retries and error handling
- [Priority Queue](./examples/priority_queue.rs) - Using job priorities
- [Progress Tracking](./examples/progress_tracking.rs) - Monitoring job progress

To run an example:

```bash
cargo run --example basic_queue
```

## Advanced Usage

### Adding a job with options

```rust
use bullrs::{Queue, JobOptions, JobPriority, Backoff};

// Configure job options
let mut options = JobOptions::default();
options.priority = Some(JobPriority::High);
options.delay = Some(10000); // 10 seconds
options.attempts = Some(3);
options.backoff = Some(Backoff {
    strategy_type: "exponential".to_string(),
    delay: 1000,
});

// Add job with options
let job = queue.add(
    "email-job",
    json!({
        "to": "user@example.com",
        "subject": "Hello",
        "body": "Hello from BullRs!"
    }),
    Some(options),
).await?;
```

### Creating repeatable jobs

```rust
use bullrs::{RepeatOptions, RepeatPattern};

// Repeat job every 30 seconds
let repeat_options = RepeatOptions {
    pattern: RepeatPattern::Every { milliseconds: 30000 },
    limit: Some(10), // Repeat 10 times
    start_time: None,
    end_time: None,
};

// Add repeatable job
let job = queue.add_repeatable_job(
    "scheduled-task",
    json!({ "task": "cleanup" }),
    repeat_options,
).await?;
```

### Using cron patterns

```rust
// Run job at 8:00 AM every day
let cron_options = RepeatOptions {
    pattern: RepeatPattern::Cron {
        expression: "0 8 * * *".to_string()
    },
    limit: None, // No limit
    start_time: None,
    end_time: None,
};

let job = queue.add_repeatable_job(
    "daily-report",
    json!({ "type": "daily-summary" }),
    cron_options,
).await?;
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

[MIT](LICENSE)
