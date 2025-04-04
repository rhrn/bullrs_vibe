# BullRs Patterns

This document describes common patterns and best practices when using BullRs.

## Table of Contents

- [Producer-Consumer](#producer-consumer)
- [Reusing Redis Connections](#reusing-redis-connections)
- [Concurrency](#concurrency)
- [Rate Limiting](#rate-limiting)
- [Priority Queue](#priority-queue)
- [Delayed Jobs](#delayed-jobs)
- [Repeatable Jobs](#repeatable-jobs)
- [Error Handling and Retries](#error-handling-and-retries)
- [Job Completion Acknowledgment](#job-completion-acknowledgment)
- [Queue Management](#queue-management)
- [Monitoring](#monitoring)

## Producer-Consumer

The most basic BullRs pattern is the producer-consumer pattern, where one part of your application adds jobs to the queue (producer) and another part processes those jobs (consumer).

### Producer Example

```rust
use bullrs::{Queue, QueueOptions};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn producer() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue_options = QueueOptions::default();
    let queue = Queue::new("my-queue", client, queue_options).await?;
    
    // Add jobs
    for i in 0..10 {
        let job = queue.add(
            "my-job",
            json!({
                "job_number": i,
                "data": format!("data-{}", i)
            }),
            None,
        ).await?;
        
        println!("Added job: {}", job.id);
    }
    
    Ok(())
}
```

### Consumer Example

```rust
use bullrs::{Queue, QueueOptions, Worker, ScriptLoader};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn consumer() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Initialize script loader
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Define a job processor
    let processor = Arc::new(|job| {
        println!("Processing job: {}", job.id);
        
        // Process job data
        let job_number = job.data.get("job_number")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
            
        // Return result
        Ok(json!({ "processed": true, "job_number": job_number }))
    });
    
    // Create worker
    let worker = Worker::new(
        client.clone(),
        "my-queue",
        5, // concurrency
        processor,
        script_loader,
    ).await?;
    
    // Keep the worker running
    println!("Worker is running. Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    
    // Stop worker
    worker.stop().await?;
    println!("Worker stopped");
    
    Ok(())
}
```

## Reusing Redis Connections

It's generally a good practice to reuse Redis connections across your application. In BullRs, you can create a single Redis client and share it among multiple queues.

```rust
use bullrs::{Queue, QueueOptions};
use redis::Client;
use std::sync::Arc;

async fn setup_queues() -> Result<(), Box<dyn std::error::Error>> {
    // Create a single Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create multiple queues sharing the same connection
    let email_queue = Queue::new("email-queue", client.clone(), QueueOptions::default()).await?;
    let sms_queue = Queue::new("sms-queue", client.clone(), QueueOptions::default()).await?;
    let notification_queue = Queue::new("notification-queue", client.clone(), QueueOptions::default()).await?;
    
    // Use queues...
    
    Ok(())
}
```

## Concurrency

BullRs allows you to control the concurrency of job processing by setting the concurrency parameter when creating a Worker.

```rust
// Create a worker that processes up to 10 jobs concurrently
let worker = Worker::new(
    client.clone(),
    "my-queue",
    10, // process up to 10 jobs concurrently
    processor,
    script_loader,
).await?;
```

## Rate Limiting

You can implement rate limiting by using a combination of delayed jobs and the queue's `pause` and `resume` methods.

```rust
use bullrs::{Queue, QueueOptions, JobOptions};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time;

async fn rate_limited_processing() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("rate-limited-queue", client, QueueOptions::default()).await?;
    
    // Process 100 jobs in batches of 10 with a rate limit
    for i in 0..100 {
        // Add job
        queue.add("rate-limited-job", json!({ "index": i }), None).await?;
        
        // After every 10 jobs, pause the queue for 1 second
        if i > 0 && i % 10 == 0 {
            queue.pause().await?;
            time::sleep(Duration::from_secs(1)).await;
            queue.resume().await?;
        }
    }
    
    Ok(())
}
```

## Priority Queue

BullRs supports job priorities, allowing you to process more important jobs first.

```rust
use bullrs::{Queue, QueueOptions, JobOptions, JobPriority};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn add_prioritized_jobs() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("priority-queue", client, QueueOptions::default()).await?;
    
    // Add a critical priority job
    let critical_options = JobOptions {
        priority: Some(JobPriority::Critical),
        ..JobOptions::default()
    };
    
    queue.add(
        "critical-job",
        json!({ "importance": "critical" }),
        Some(critical_options),
    ).await?;
    
    // Add a normal priority job
    let normal_options = JobOptions {
        priority: Some(JobPriority::Normal),
        ..JobOptions::default()
    };
    
    queue.add(
        "normal-job",
        json!({ "importance": "normal" }),
        Some(normal_options),
    ).await?;
    
    // Add a low priority job
    let low_options = JobOptions {
        priority: Some(JobPriority::Low),
        ..JobOptions::default()
    };
    
    queue.add(
        "low-job",
        json!({ "importance": "low" }),
        Some(low_options),
    ).await?;
    
    Ok(())
}
```

## Delayed Jobs

You can schedule jobs to be processed at a later time by using the delay option.

```rust
use bullrs::{Queue, QueueOptions, JobOptions};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn schedule_delayed_job() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("delayed-queue", client, QueueOptions::default()).await?;
    
    // Schedule a job to run after 30 seconds
    let options = JobOptions {
        delay: Some(30000), // 30 seconds in milliseconds
        ..JobOptions::default()
    };
    
    queue.add(
        "delayed-job",
        json!({ "scheduled": true }),
        Some(options),
    ).await?;
    
    Ok(())
}
```

## Repeatable Jobs

BullRs supports repeatable jobs that run at specified intervals.

### Using Fixed Interval Pattern

```rust
use bullrs::{Queue, QueueOptions, RepeatOptions, RepeatPattern};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn add_repeating_job() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("repeat-queue", client, QueueOptions::default()).await?;
    
    // Repeat job every 5 minutes
    let repeat_options = RepeatOptions {
        pattern: RepeatPattern::Every { milliseconds: 300000 }, // 5 minutes
        limit: None, // no limit
        start_time: None,
        end_time: None,
    };
    
    queue.add_repeatable_job(
        "hourly-report",
        json!({ "task": "generate-report" }),
        repeat_options,
    ).await?;
    
    Ok(())
}
```

### Using Cron Pattern

```rust
use bullrs::{Queue, QueueOptions, RepeatOptions, RepeatPattern};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn add_cron_job() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("cron-queue", client, QueueOptions::default()).await?;
    
    // Run job every day at midnight
    let repeat_options = RepeatOptions {
        pattern: RepeatPattern::Cron { 
            expression: "0 0 * * *".to_string() 
        },
        limit: None,
        start_time: None,
        end_time: None,
    };
    
    queue.add_repeatable_job(
        "daily-backup",
        json!({ "task": "backup-database" }),
        repeat_options,
    ).await?;
    
    Ok(())
}
```

## Error Handling and Retries

BullRs allows you to configure automatic retries for failed jobs.

```rust
use bullrs::{Queue, QueueOptions, JobOptions, Backoff};
use redis::Client;
use serde_json::json;
use std::sync::Arc;

async fn add_job_with_retries() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("retry-queue", client, QueueOptions::default()).await?;
    
    // Configure job to retry up to 3 times with exponential backoff
    let options = JobOptions {
        attempts: Some(3),
        backoff: Some(Backoff {
            strategy_type: "exponential".to_string(),
            delay: 1000, // Start with 1 second delay
        }),
        ..JobOptions::default()
    };
    
    queue.add(
        "retry-job",
        json!({ "data": "important-task" }),
        Some(options),
    ).await?;
    
    Ok(())
}
```

## Job Completion Acknowledgment

In many cases, you might want to wait for a job to complete. BullRs allows you to get a job by its ID and check its status.

```rust
use bullrs::{Queue, QueueOptions, JobState};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

async fn wait_for_job_completion() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("ack-queue", client, QueueOptions::default()).await?;
    
    // Add a job
    let job = queue.add("ack-job", json!({ "task": "important" }), None).await?;
    let job_id = job.id.clone();
    
    println!("Job added with ID: {}", job_id);
    
    // Wait for job completion
    loop {
        // Get jobs with completed status
        let completed_jobs = queue.get_jobs("completed", 0, 100).await?;
        
        // Check if our job is in the completed list
        if completed_jobs.iter().any(|j| j.id == job_id) {
            println!("Job completed successfully!");
            break;
        }
        
        // Get jobs with failed status
        let failed_jobs = queue.get_jobs("failed", 0, 100).await?;
        
        // Check if our job is in the failed list
        if failed_jobs.iter().any(|j| j.id == job_id) {
            println!("Job failed!");
            break;
        }
        
        // Wait and check again
        sleep(Duration::from_secs(1)).await;
    }
    
    Ok(())
}
```

## Queue Management

BullRs provides methods for managing queue state and jobs.

### Pausing and Resuming Queues

```rust
async fn manage_queue() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("managed-queue", client, QueueOptions::default()).await?;
    
    // Pause queue during maintenance
    queue.pause().await?;
    println!("Queue paused for maintenance");
    
    // Perform maintenance tasks...
    
    // Resume queue
    queue.resume().await?;
    println!("Queue resumed");
    
    Ok(())
}
```

### Cleaning Up Completed Jobs

```rust
async fn cleanup_jobs() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("cleanup-queue", client, QueueOptions::default()).await?;
    
    // Remove completed jobs older than 1 hour
    let one_hour_ms = 60 * 60 * 1000;
    let cleaned = queue.clean(one_hour_ms, "completed", 1000).await?;
    
    println!("Removed {} completed jobs", cleaned);
    
    Ok(())
}
```

## Monitoring

You can monitor your queues by periodically checking job counts and states.

```rust
use bullrs::{Queue, QueueOptions};
use redis::Client;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

async fn monitor_queues() -> Result<(), Box<dyn std::error::Error>> {
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    let queue = Queue::new("monitored-queue", client, QueueOptions::default()).await?;
    
    // Monitor queue every 5 seconds
    loop {
        let counts = queue.get_counts().await?;
        
        println!("Queue Stats:");
        println!("  Waiting: {}", counts.get("waiting").unwrap_or(&0));
        println!("  Active: {}", counts.get("active").unwrap_or(&0));
        println!("  Completed: {}", counts.get("completed").unwrap_or(&0));
        println!("  Failed: {}", counts.get("failed").unwrap_or(&0));
        println!("  Delayed: {}", counts.get("delayed").unwrap_or(&0));
        
        // Check for failed jobs
        if let Some(&failed) = counts.get("failed") {
            if failed > 0 {
                println!("WARNING: Found {} failed jobs!", failed);
                
                // Get details of failed jobs
                let failed_jobs = queue.get_jobs("failed", 0, 10).await?;
                for job in failed_jobs {
                    println!("  Job {} failed: {}", job.id, job.stack_trace.unwrap_or_default());
                }
            }
        }
        
        sleep(Duration::from_secs(5)).await;
    }
}
```

These patterns should help you get started with BullRs and give you ideas for how to structure your job processing applications.
