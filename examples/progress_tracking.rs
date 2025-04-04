use bullrs::{Queue, QueueOptions, Worker, ScriptLoader, Job};
use redis::Client;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// This example demonstrates job progress tracking, including how to
/// update and monitor progress within job processing.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create Redis client
    let client = Arc::new(Client::open("redis://127.0.0.1:6379")?);
    
    // Create queue
    let queue = Queue::new(
        "progress-queue", 
        client.clone(), 
        QueueOptions::default()
    ).await?;
    
    // Add a job that we'll track progress for
    println!("Adding job with progress tracking...");
    let job = queue.add(
        "progress-job",
        json!({
            "steps": 10,
            "job_name": "Data Processing Task"
        }),
        None,
    ).await?;
    
    println!("Added job with ID: {}", job.id);
    
    // Set up a worker to process the job with progress updates
    let script_loader = Arc::new(ScriptLoader::new()?);
    
    // Create channel to communicate progress updates
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(100);
    
    // Spawn task to listen for progress updates
    tokio::spawn(async move {
        println!("\nProgress monitoring started:");
        println!("-------------------------------------------------------------");
        
        while let Some((job_id, percentage, data)) = progress_rx.recv().await {
            let stage = data.get("stage").and_then(|v| v.as_str()).unwrap_or("unknown");
            
            println!("Job {} progress: {:.1}% - Stage: {}", 
                job_id, percentage, stage);
        }
        
        println!("Progress monitoring ended");
    });
    
    // Clone for the processor closure
    let progress_tx_clone = progress_tx.clone();
    
    // Define the job processor with progress updates
    let processor = Arc::new(move |mut job: Job| {
        println!("Starting job: {}", job.id);
        
        // Get the number of steps from job data
        let total_steps = job.data.get("steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;
            
        let job_name = job.data.get("job_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed Job");
            
        println!("Processing '{}' with {} steps", job_name, total_steps);
        
        // Clone for async block
        let job_id = job.id.clone();
        let tx = progress_tx_clone.clone();
        
        // Process each step with progress updates
        for step in 1..=total_steps {
            // Calculate progress percentage
            let percentage = (step as f64 / total_steps as f64) * 100.0;
            
            // Update job progress with data
            let progress_data = json!({
                "stage": format!("Step {}/{}", step, total_steps),
                "description": format!("Processing task {}", step),
                "timestamp": chrono::Utc::now().to_rfc3339()
            });
            
            if let Err(e) = job.update_progress_with_data(percentage, progress_data.clone()) {
                eprintln!("Error updating progress: {}", e);
            }
            
            // Send progress update to our monitoring channel
            let _ = tx.try_send((job_id.clone(), percentage, progress_data));
            
            // Simulate work for this step
            std::thread::sleep(Duration::from_millis(500));
        }
        
        // Return successful result
        Ok(json!({
            "completed": true,
            "job_name": job_name,
            "steps_completed": total_steps
        }))
    });
    
    // Create and start worker
    println!("\nStarting worker to process job with progress updates...");
    let worker = Worker::new(
        client.clone(),
        "progress-queue",
        1,
        processor,
        script_loader,
    ).await?;
    
    // Wait for job to complete
    println!("Waiting for job to complete...");
    sleep(Duration::from_secs(8)).await;
    
    // Get completed job to check final progress
    let completed_jobs = queue.get_jobs("completed", 0, 10).await?;
    
    println!("\nCompleted job details:");
    for job in completed_jobs {
        println!("  Job ID: {}", job.id);
        println!("  Final progress: {:.1}%", job.progress.percentage);
        
        if let Some(data) = &job.progress.data {
            if let Some(stage) = data.get("stage").and_then(|v| v.as_str()) {
                println!("  Final stage: {}", stage);
            }
            
            if let Some(desc) = data.get("description").and_then(|v| v.as_str()) {
                println!("  Description: {}", desc);
            }
        }
        
        if let Some(result) = &job.return_value {
            println!("  Result: {:?}", result);
        }
    }
    
    // Stop the worker
    worker.stop().await?;
    println!("Worker stopped");
    
    // Close the progress channel
    drop(progress_tx);
    
    // Wait a moment for the progress monitor to finish
    sleep(Duration::from_millis(100)).await;
    
    Ok(())
}
