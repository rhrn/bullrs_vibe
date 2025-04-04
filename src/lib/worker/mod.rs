use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::{Client, Connection, RedisError};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc, RwLock};
use tokio::time;

use crate::commands::script_loader::ScriptLoader;
use crate::errors::BullRsError;
use crate::job::{Job, JobState};

/// Processor type for processing jobs
pub type JobProcessor = Arc<dyn Fn(Job) -> Result<Value, String> + Send + Sync>;

/// Worker responsible for processing jobs from a queue
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
    active_jobs: RwLock<HashMap<String, Job>>,
}

impl Worker {
    /// Create a new worker
    pub async fn new(
        client: Arc<Client>,
        queue_name: &str,
        concurrency: usize,
        processor: JobProcessor,
        script_loader: Arc<ScriptLoader>,
    ) -> Result<Self, BullRsError> {
        // Create stop channel
        let (tx, rx) = mpsc::channel::<()>(1);
        
        let worker = Worker {
            client,
            queue_name: queue_name.to_string(),
            concurrency,
            processor,
            script_loader,
            running: RwLock::new(false),
            stop_channel: (tx, Mutex::new(rx)),
            active_jobs: RwLock::new(HashMap::new()),
        };
        
        // Start worker
        worker.start().await?;
        
        Ok(worker)
    }
    
    /// Start the worker
    async fn start(&self) -> Result<(), BullRsError> {
        let mut running = self.running.write().await;
        
        if *running {
            return Ok(());
        }
        
        *running = true;
        
        // Start job processing tasks
        for i in 0..self.concurrency {
            let client = self.client.clone();
            let queue_name = self.queue_name.clone();
            let processor = self.processor.clone();
            let script_loader = self.script_loader.clone();
            let stop_rx = self.stop_channel.1.lock().await.try_recv();
            let active_jobs = self.active_jobs.clone();
            
            tokio::spawn(async move {
                loop {
                    // Check if stop signal received
                    if let Ok(_) = stop_rx {
                        break;
                    }
                    
                    // Try to get a job
                    let job_result = Self::get_next_job(&client, &queue_name, &script_loader).await;
                    
                    match job_result {
                        Ok(Some(mut job)) => {
                            // Store job in active jobs
                            {
                                let mut jobs = active_jobs.write().await;
                                jobs.insert(job.id.clone(), job.clone());
                            }
                            
                            // Process job
                            let result = Self::process_job(&mut job, &processor).await;
                            
                            // Complete job
                            let complete_result = Self::complete_job(
                                &client,
                                &queue_name,
                                &mut job,
                                result,
                                &script_loader,
                            ).await;
                            
                            // Log any errors
                            if let Err(e) = complete_result {
                                eprintln!("Error completing job {}: {}", job.id, e);
                            }
                            
                            // Remove job from active jobs
                            {
                                let mut jobs = active_jobs.write().await;
                                jobs.remove(&job.id);
                            }
                        }
                        Ok(None) => {
                            // No job available, wait before trying again
                            time::sleep(Duration::from_millis(500)).await;
                        }
                        Err(e) => {
                            eprintln!("Error getting next job: {}", e);
                            time::sleep(Duration::from_millis(1000)).await;
                        }
                    }
                }
            });
        }
        
        Ok(())
    }
    
    /// Get the next job from the queue
    async fn get_next_job(
        client: &Client,
        queue_name: &str,
        script_loader: &ScriptLoader,
    ) -> Result<Option<Job>, BullRsError> {
        let mut conn = client.get_connection()
            .map_err(|e| BullRsError::RedisError(e))?;
        
        // Execute moveToActive script
        let result: Option<String> = redis::cmd("EVAL")
            .arg("-- Implementation of moveToActive script")
            .arg(3) // Number of KEYS
            .arg(format!("{}:wait", queue_name))
            .arg(format!("{}:active", queue_name))
            .arg(format!("{}:paused", queue_name))
            .arg("1") // ARGV[1]: Max number of jobs to fetch
            .query(&mut conn)
            .map_err(|e| BullRsError::RedisError(e))?;
        
        match result {
            Some(job_id) => {
                // Get job data
                let job = Job::from_id(queue_name, &job_id, &mut conn).await?;
                
                if let Some(mut job) = job {
                    // Mark job as active
                    job.mark_as_active();
                    
                    // Update job data in Redis
                    let job_data = serde_json::to_string(&job)
                        .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
                    
                    redis::cmd("SET")
                        .arg(format!("{}:job:{}", queue_name, job.id))
                        .arg(job_data)
                        .query(&mut conn)
                        .map_err(|e| BullRsError::RedisError(e))?;
                    
                    Ok(Some(job))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
    
    /// Process a job
    async fn process_job(
        job: &mut Job,
        processor: &JobProcessor,
    ) -> Result<Value, String> {
        // Process the job
        match processor(job.clone()) {
            Ok(result) => Ok(result),
            Err(err) => Err(err),
        }
    }
    
    /// Complete a job (mark as completed or failed)
    async fn complete_job(
        client: &Client,
        queue_name: &str,
        job: &mut Job,
        result: Result<Value, String>,
        script_loader: &ScriptLoader,
    ) -> Result<(), BullRsError> {
        let mut conn = client.get_connection()
            .map_err(|e| BullRsError::RedisError(e))?;
        
        match result {
            Ok(return_value) => {
                // Mark job as completed
                job.mark_as_completed(Some(return_value.clone()));
                
                // Update job data
                let job_data = serde_json::to_string(&job)
                    .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
                
                // Execute moveToFinished script
                redis::cmd("EVAL")
                    .arg("-- Implementation of moveToFinished script")
                    .arg(5) // Number of KEYS
                    .arg(format!("{}:", queue_name))
                    .arg(format!("{}:active", queue_name))
                    .arg(format!("{}:completed", queue_name))
                    .arg(format!("{}:failed", queue_name))
                    .arg(format!("{}:stalled", queue_name))
                    .arg(job.id.clone())
                    .arg(job_data)
                    .arg("completed")
                    .arg(job.opts.remove_on_complete.unwrap_or(false).to_string())
                    .query(&mut conn)
                    .map_err(|e| BullRsError::RedisError(e))?;
            }
            Err(err) => {
                // Mark job as failed
                job.mark_as_failed(&err, None);
                
                // Update job data
                let job_data = serde_json::to_string(&job)
                    .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
                
                // Execute moveToFinished script
                redis::cmd("EVAL")
                    .arg("-- Implementation of moveToFinished script")
                    .arg(5) // Number of KEYS
                    .arg(format!("{}:", queue_name))
                    .arg(format!("{}:active", queue_name))
                    .arg(format!("{}:completed", queue_name))
                    .arg(format!("{}:failed", queue_name))
                    .arg(format!("{}:stalled", queue_name))
                    .arg(job.id.clone())
                    .arg(job_data)
                    .arg("failed")
                    .arg(job.opts.remove_on_fail.unwrap_or(false).to_string())
                    .query(&mut conn)
                    .map_err(|e| BullRsError::RedisError(e))?;
                
                // Check if job can be retried
                if job.can_retry() {
                    // Retry job after backoff delay
                    let delay = match &job.opts.backoff {
                        Some(backoff) => backoff.delay,
                        None => 1000, // Default 1 second delay
                    };
                    
                    // Execute retry script
                    redis::cmd("EVAL")
                        .arg("-- Implementation of retryJob script")
                        .arg(3) // Number of KEYS
                        .arg(format!("{}:failed", queue_name))
                        .arg(format!("{}:wait", queue_name))
                        .arg(format!("{}:delayed", queue_name))
                        .arg(job.id.clone())
                        .arg(SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64 + delay)
                        .query(&mut conn)
                        .map_err(|e| BullRsError::RedisError(e))?;
                }
            }
        }
        
        Ok(())
    }
    
    /// Stop the worker
    pub async fn stop(&self) -> Result<(), BullRsError> {
        let mut running = self.running.write().await;
        
        if !*running {
            return Ok(());
        }
        
        // Send stop signal
        if let Err(e) = self.stop_channel.0.send(()).await {
            eprintln!("Error sending stop signal: {}", e);
        }
        
        *running = false;
        
        Ok(())
    }
    
    /// Get the number of active jobs
    pub async fn active_job_count(&self) -> usize {
        let jobs = self.active_jobs.read().await;
        jobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    
    // Helper function to create a test Redis client
    fn create_test_client() -> Arc<Client> {
        let client = Client::open("redis://127.0.0.1:6379").unwrap();
        Arc::new(client)
    }
    
    #[tokio::test]
    async fn test_worker_creation() {
        // This test requires a Redis server running
        let client_result = Client::open("redis://127.0.0.1:6379");
        
        if let Ok(client) = client_result {
            // Set up script loader
            let script_loader = ScriptLoader::new().unwrap();
            
            // Define a simple processor
            let processor: JobProcessor = Arc::new(|job| {
                println!("Processing job: {}", job.id);
                Ok(json!("test result"))
            });
            
            // Create worker
            let worker_result = Worker::new(
                Arc::new(client),
                "test-worker-queue",
                1,
                processor,
                Arc::new(script_loader),
            ).await;
            
            assert!(worker_result.is_ok());
            
            // Test worker state
            if let Ok(worker) = worker_result {
                let running = worker.running.read().await;
                assert!(*running);
                
                // Stop worker
                drop(running);
                let stop_result = worker.stop().await;
                assert!(stop_result.is_ok());
                
                // Verify worker stopped
                let running = worker.running.read().await;
                assert!(!*running);
            }
        }
    }
}
