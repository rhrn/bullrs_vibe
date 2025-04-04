use bullrs::{Queue, QueueOptions, Worker, ScriptLoader, RepeatOptions, RepeatPattern};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// This example demonstrates how to create and manage repeatable jobs.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue = Queue::new(
        "repeatable-queue", 
        client.clone(), 
        QueueOptions::default()
    ).await?;
    
    // Define a job that repeats every 5 seconds
    let every_pattern = RepeatOptions {
        pattern: RepeatPattern::Every { milliseconds: 5000 },
        limit: Some(3), // Repeat only 3 times
        start_time: None,
        end_time: None,
    };
    
    // Add the repeatable job
    println!("Adding a job that repeats every 5 seconds (3 times)...");
    let job1 = queue.add_repeatable_job(
        "repeat-every",
        json!({ "type": "repeating_every_5s" }),
        every_pattern,
    ).await?;
    
    println!("Added repeatable job with ID: {}", job1.id);
    
    // Define a job with cron-like pattern (every minute)
    let cron_pattern = RepeatOptions {
        pattern: RepeatPattern::Cron { 
            expression: "* * * * *".to_string() 
        },
        limit: Some(2), // Repeat only twice
        start_time: None,
        end_time: None,
    };
    
    // Add the cron job
    println!("Adding a job that repeats every minute (2 times)...");
    let job2 = queue.add_repeatable_job(
        "repeat-cron",
        json!({ "type": "repeating_cron_minute" }),
        cron_pattern,
    ).await?;
    
    println!("Added cron job with ID: {}", job2.id);
    
    // Set up a worker to process the jobs
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    let processor = Arc::new(|job| {
        let job_type = job.data.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
        let repeat_count = job.progress.repeat_count.unwrap_or(0);
        
        println!("Processing repeatable job: {} (type: {}, repeat count: {})", 
            job.id, job_type, repeat_count);
        
        // Return success
        Ok(json!({ "processed": true }))
    });
    
    // Create and start worker
    println!("\nStarting worker to process repeatable jobs...");
    let worker = Worker::new(
        client.clone(),
        "repeatable-queue",
        1,
        processor,
        script_loader,
    ).await?;
    
    // Wait for some job executions to occur
    println!("Waiting for jobs to be processed (20 seconds)...");
    
    // Wait for the first job to be processed 3 times (every 5 seconds)
    // and for the cron job to potentially run
    sleep(Duration::from_secs(20)).await;
    
    // List all repeatable jobs
    println!("\nListing repeatable jobs:");
    let repeatable_jobs = queue.get_repeatable_jobs().await?;
    
    if repeatable_jobs.is_empty() {
        println!("No repeatable jobs found (all may have completed their repetitions)");
    } else {
        for (key, options) in repeatable_jobs {
            println!("Repeatable job: {}", key);
            println!("  Pattern: {:?}", options.pattern);
            println!("  Limit: {:?}", options.limit);
        }
    }
    
    // Get job counts
    let counts = queue.get_counts().await?;
    println!("\nQueue statistics:");
    println!("  Waiting: {}", counts.get("waiting").unwrap_or(&0));
    println!("  Active: {}", counts.get("active").unwrap_or(&0));
    println!("  Completed: {}", counts.get("completed").unwrap_or(&0));
    println!("  Failed: {}", counts.get("failed").unwrap_or(&0));
    
    // Cleanup: stop worker
    worker.stop().await?;
    println!("Worker stopped");
    
    Ok(())
}
