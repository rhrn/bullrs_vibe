use bullrs::{Queue, QueueOptions, Job, Worker, ScriptLoader};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// This example demonstrates the basic usage of BullRs
/// with a simple producer-consumer pattern.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue with default options
    let queue = Queue::new(
        "example-queue", 
        client.clone(), 
        QueueOptions::default()
    ).await?;
    
    // Producer: Add jobs to the queue
    println!("Adding jobs to the queue...");
    
    for i in 1..=5 {
        let job = queue.add(
            "example-job",
            json!({
                "job_index": i,
                "message": format!("This is job number {}", i)
            }),
            None,
        ).await?;
        
        println!("Added job with ID: {}", job.id);
    }
    
    // Consumer: Set up a worker to process the jobs
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Define the job processor function
    let processor = Arc::new(|job: Job| {
        println!("Processing job ID: {}", job.id);
        println!("Job data: {:?}", job.data);
        
        // Simulate some work
        std::thread::sleep(Duration::from_secs(1));
        
        // Return a result
        Ok(json!({
            "processed": true,
            "result": format!("Completed job {}", job.id)
        }))
    });
    
    // Create and start the worker
    println!("\nStarting worker to process jobs...");
    let worker = Worker::new(
        client.clone(),
        "example-queue",
        2, // Process up to 2 jobs concurrently
        processor,
        script_loader,
    ).await?;
    
    // Let the worker run for a few seconds to process all jobs
    println!("Waiting for jobs to be processed...");
    sleep(Duration::from_secs(10)).await;
    
    // Get job counts to verify processing
    let counts = queue.get_counts().await?;
    println!("\nQueue statistics:");
    println!("  Waiting: {}", counts.get("waiting").unwrap_or(&0));
    println!("  Active: {}", counts.get("active").unwrap_or(&0));
    println!("  Completed: {}", counts.get("completed").unwrap_or(&0));
    println!("  Failed: {}", counts.get("failed").unwrap_or(&0));
    
    // Stop the worker
    worker.stop().await?;
    println!("Worker stopped");
    
    Ok(())
}
