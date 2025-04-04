use bullrs::{Queue, QueueOptions, Worker, ScriptLoader, JobOptions, JobPriority};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// This example demonstrates job prioritization, showing how jobs
/// with higher priorities are processed first.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue = Queue::new(
        "priority-queue", 
        client.clone(), 
        QueueOptions::default()
    ).await?;
    
    // Add jobs with different priorities
    println!("Adding jobs with different priorities...");
    
    // Add low priority jobs
    for i in 1..=3 {
        let mut options = JobOptions::default();
        options.priority = Some(JobPriority::Low);
        
        let job = queue.add(
            "low-priority-job",
            json!({
                "index": i,
                "priority": "low"
            }),
            Some(options),
        ).await?;
        
        println!("Added LOW priority job with ID: {}", job.id);
    }
    
    // Add normal priority jobs
    for i in 1..=3 {
        let mut options = JobOptions::default();
        options.priority = Some(JobPriority::Normal);
        
        let job = queue.add(
            "normal-priority-job",
            json!({
                "index": i,
                "priority": "normal"
            }),
            Some(options),
        ).await?;
        
        println!("Added NORMAL priority job with ID: {}", job.id);
    }
    
    // Add high priority jobs
    for i in 1..=3 {
        let mut options = JobOptions::default();
        options.priority = Some(JobPriority::High);
        
        let job = queue.add(
            "high-priority-job",
            json!({
                "index": i,
                "priority": "high"
            }),
            Some(options),
        ).await?;
        
        println!("Added HIGH priority job with ID: {}", job.id);
    }
    
    // Add one critical priority job
    let mut options = JobOptions::default();
    options.priority = Some(JobPriority::Critical);
    
    let job = queue.add(
        "critical-priority-job",
        json!({
            "index": 1,
            "priority": "critical"
        }),
        Some(options),
    ).await?;
    
    println!("Added CRITICAL priority job with ID: {}", job.id);
    
    // Set up a worker to process the jobs
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Define the processor that will record the order of job processing
    let processed_jobs = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let processed_jobs_clone = processed_jobs.clone();
    
    let processor = Arc::new(move |job| {
        let priority = job.data.get("priority").and_then(|v| v.as_str()).unwrap_or("unknown");
        let index = job.data.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
        
        println!("Processing {} priority job {}", priority, job.id);
        
        // Record the job in our processing order
        let job_info = format!("{} (index: {})", priority, index);
        
        // Clone to capture in async block
        let jobs = processed_jobs_clone.clone();
        
        // Add to our processing order list
        tokio::spawn(async move {
            let mut jobs_lock = jobs.lock().await;
            jobs_lock.push(job_info);
        });
        
        // Simulate work
        std::thread::sleep(Duration::from_millis(200));
        
        Ok(json!({ "processed": true }))
    });
    
    // Create and start the worker
    println!("\nStarting worker to process jobs in priority order...");
    let worker = Worker::new(
        client.clone(),
        "priority-queue",
        1, // Only 1 concurrent job to clearly see priority order
        processor,
        script_loader,
    ).await?;
    
    // Wait for all jobs to be processed
    println!("Waiting for jobs to be processed...");
    sleep(Duration::from_secs(5)).await;
    
    // Display the order in which jobs were processed
    let processing_order = processed_jobs.lock().await;
    
    println!("\nJobs were processed in the following order:");
    for (i, job) in processing_order.iter().enumerate() {
        println!("  {}. {}", i + 1, job);
    }
    
    // Verify that higher priority jobs were processed first
    
    // Get queue statistics
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
