use bullrs::{Queue, QueueOptions, Worker, ScriptLoader, JobOptions, Backoff};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// This example demonstrates error handling and automatic retry behavior
/// for failed jobs in BullRs.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue = Queue::new(
        "retry-queue", 
        client.clone(), 
        QueueOptions::default()
    ).await?;
    
    // Add a job that will fail on first attempt but succeed on retry
    let mut retry_options = JobOptions::default();
    retry_options.attempts = Some(3); // Allow up to 3 attempts
    retry_options.backoff = Some(Backoff {
        strategy_type: "fixed".to_string(),
        delay: 2000, // 2 seconds delay between retries
    });
    
    println!("Adding job with retry configuration...");
    let job = queue.add(
        "retry-job",
        json!({
            "should_fail": true,
            "fail_until_attempt": 2  // Succeed on the 2nd attempt
        }),
        Some(retry_options),
    ).await?;
    
    println!("Added job with ID: {} (max attempts: {})", 
        job.id, job.max_attempts);
    
    // Set up a worker to process the job
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Define a processor that simulates failure
    let processor = Arc::new(|job| {
        let should_fail = job.data.get("should_fail")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
            
        let fail_until_attempt = job.data.get("fail_until_attempt")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        
        println!("Processing job ID: {} (attempt {}/{})", 
            job.id, job.attempts_made, job.max_attempts);
        
        if should_fail && job.attempts_made < fail_until_attempt {
            println!("  This attempt will fail!");
            return Err("Simulated failure".to_string());
        }
        
        println!("  Job processed successfully!");
        Ok(json!({ "success": true }))
    });
    
    // Create worker to process jobs
    println!("\nStarting worker to process jobs with retry logic...");
    let worker = Worker::new(
        client.clone(),
        "retry-queue",
        1,
        processor,
        script_loader,
    ).await?;
    
    // Wait for processing to complete (with retries)
    println!("Waiting for job processing and retries...");
    sleep(Duration::from_secs(10)).await;
    
    // Check final job status
    let completed_jobs = queue.get_jobs("completed", 0, 10).await?;
    let failed_jobs = queue.get_jobs("failed", 0, 10).await?;
    
    println!("\nJob processing results:");
    if !completed_jobs.is_empty() {
        for job in completed_jobs {
            println!("  Job {} completed successfully after {} attempts", 
                job.id, job.attempts_made);
        }
    }
    
    if !failed_jobs.is_empty() {
        for job in failed_jobs {
            println!("  Job {} failed after {} attempts", 
                job.id, job.attempts_made);
            println!("  Error: {}", job.stack_trace.unwrap_or_default());
        }
    }
    
    // Get job counts
    let counts = queue.get_counts().await?;
    println!("\nQueue statistics:");
    println!("  Waiting: {}", counts.get("waiting").unwrap_or(&0));
    println!("  Active: {}", counts.get("active").unwrap_or(&0));
    println!("  Completed: {}", counts.get("completed").unwrap_or(&0));
    println!("  Failed: {}", counts.get("failed").unwrap_or(&0));
    
    // Stop worker
    worker.stop().await?;
    println!("Worker stopped");
    
    Ok(())
}
