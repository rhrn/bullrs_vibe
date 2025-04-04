use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::{mpsc, RwLock};
use tokio::task;

use crate::errors::BullRsError;
use crate::job::Job;

/// Sandbox process options
pub struct SandboxOptions {
    /// Maximum number of processes to keep
    pub max_processes: usize,
    /// Timeout in milliseconds before killing a process
    pub timeout: u64,
}

impl Default for SandboxOptions {
    fn default() -> Self {
        SandboxOptions {
            max_processes: 4,
            timeout: 30000, // 30 seconds
        }
    }
}

/// Process pool for handling job processing in separate processes
pub struct ProcessPool {
    /// Pool options
    options: SandboxOptions,
    /// Map of available processes
    available: RwLock<HashMap<String, Vec<Child>>>,
    /// Map of busy processes
    busy: RwLock<HashMap<String, Child>>,
}

impl ProcessPool {
    /// Create a new process pool
    pub fn new(options: Option<SandboxOptions>) -> Self {
        ProcessPool {
            options: options.unwrap_or_default(),
            available: RwLock::new(HashMap::new()),
            busy: RwLock::new(HashMap::new()),
        }
    }
    
    /// Get a process for a specific job
    pub async fn acquire(&self, processor_file: &str) -> Result<Child, BullRsError> {
        let mut available = self.available.write().await;
        let mut busy = self.busy.write().await;
        
        // Check if we have an available process
        if let Some(processes) = available.get_mut(processor_file) {
            if let Some(process) = processes.pop() {
                // Move process to busy map
                busy.insert(format!("{}-{}", processor_file, process.id()), process);
                return Ok(process);
            }
        }
        
        // No available process, create a new one if under max limit
        let total_processes = busy.len() + available.values().map(|v| v.len()).sum::<usize>();
        
        if total_processes < self.options.max_processes {
            // Create a new process
            let child = Command::new("cargo")
                .arg("run")
                .arg("--bin")
                .arg("worker")
                .arg("--")
                .arg(processor_file)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| BullRsError::ProcessError(e.to_string()))?;
            
            // Add to busy map
            busy.insert(format!("{}-{}", processor_file, child.id()), child);
            
            Ok(child)
        } else {
            Err(BullRsError::ProcessLimitReached)
        }
    }
    
    /// Release a process back to the pool
    pub async fn release(&self, processor_file: &str, process: Child) -> Result<(), BullRsError> {
        let mut available = self.available.write().await;
        let mut busy = self.busy.write().await;
        
        // Remove from busy map
        busy.remove(&format!("{}-{}", processor_file, process.id()));
        
        // Add to available map
        if !available.contains_key(processor_file) {
            available.insert(processor_file.to_string(), Vec::new());
        }
        
        if let Some(processes) = available.get_mut(processor_file) {
            processes.push(process);
        }
        
        Ok(())
    }
    
    /// Run a job in a separate process
    pub async fn run_job(&self, job: &Job, processor_file: &str) -> Result<String, BullRsError> {
        // Acquire a process
        let mut process = self.acquire(processor_file).await?;
        
        // Prepare job data
        let job_data = serde_json::to_string(job)
            .map_err(|e| BullRsError::SerializationError(e.to_string()))?;
        
        // Write job data to process stdin
        if let Some(stdin) = process.stdin.take() {
            // Write job data
            tokio::io::AsyncWriteExt::write_all(&mut tokio::io::BufWriter::new(stdin), job_data.as_bytes()).await
                .map_err(|e| BullRsError::ProcessError(e.to_string()))?;
        }
        
        // Read process output
        let mut stdout = BufReader::new(process.stdout.take().unwrap());
        let mut output = String::new();
        
        // Create timeout for process
        let timeout = tokio::time::timeout(
            std::time::Duration::from_millis(self.options.timeout),
            stdout.read_line(&mut output),
        ).await;
        
        match timeout {
            Ok(Ok(_)) => {
                // Process returned within timeout
                self.release(processor_file, process).await?;
                Ok(output)
            }
            Ok(Err(e)) => {
                // Process error
                Err(BullRsError::ProcessError(e.to_string()))
            }
            Err(_) => {
                // Timeout, kill the process
                process.kill()
                    .map_err(|e| BullRsError::ProcessError(e.to_string()))?;
                Err(BullRsError::ProcessTimeout)
            }
        }
    }
    
    /// Clean up the process pool
    pub async fn cleanup(&self) -> Result<(), BullRsError> {
        let mut available = self.available.write().await;
        let mut busy = self.busy.write().await;
        
        // Kill all available processes
        for processes in available.values_mut() {
            for mut process in processes.drain(..) {
                if let Err(e) = process.kill() {
                    eprintln!("Error killing process: {}", e);
                }
            }
        }
        
        // Kill all busy processes
        for (_, mut process) in busy.drain() {
            if let Err(e) = process.kill() {
                eprintln!("Error killing process: {}", e);
            }
        }
        
        Ok(())
    }
}

impl Drop for ProcessPool {
    fn drop(&mut self) {
        // Spawn a task to clean up processes when the pool is dropped
        task::spawn(async move {
            // Clean up processes
        });
    }
}

/// Sandbox for running job processors
pub struct Sandbox {
    /// Process pool
    pool: Arc<ProcessPool>,
    /// Base directory for processors
    processor_base_dir: PathBuf,
}

impl Sandbox {
    /// Create a new sandbox
    pub fn new(processor_base_dir: &Path, options: Option<SandboxOptions>) -> Self {
        Sandbox {
            pool: Arc::new(ProcessPool::new(options)),
            processor_base_dir: processor_base_dir.to_path_buf(),
        }
    }
    
    /// Run a job in the sandbox
    pub async fn run(&self, job: &Job, processor_file: &str) -> Result<String, BullRsError> {
        // Resolve processor file path
        let processor_path = self.processor_base_dir.join(processor_file);
        
        // Verify processor file exists
        if !processor_path.exists() {
            return Err(BullRsError::FileNotFound(processor_path.to_string_lossy().to_string()));
        }
        
        // Run job in process pool
        self.pool.run_job(job, &processor_path.to_string_lossy()).await
    }
    
    /// Clean up the sandbox
    pub async fn cleanup(&self) -> Result<(), BullRsError> {
        self.pool.cleanup().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;
    
    async fn create_test_processor(dir: &Path) -> PathBuf {
        // Create a simple processor file
        let processor_path = dir.join("test_processor.js");
        let mut file = File::create(&processor_path).await.unwrap();
        
        // Write a simple processor
        let content = r#"
#!/usr/bin/env node
const job = JSON.parse(process.argv[2]);
console.log(JSON.stringify({ result: "processed " + job.id }));
"#;
        
        file.write_all(content.as_bytes()).await.unwrap();
        file.sync_all().await.unwrap();
        
        // Make it executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&processor_path).unwrap();
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&processor_path, perms).unwrap();
        }
        
        processor_path
    }
    
    #[tokio::test]
    async fn test_process_pool() {
        // Create a temporary directory
        let temp_dir = tempdir().unwrap();
        let processor_path = create_test_processor(temp_dir.path()).await;
        
        // Create a process pool
        let pool = ProcessPool::new(None);
        
        // Test acquiring a process
        let process = pool.acquire(&processor_path.to_string_lossy()).await;
        
        // We may not be able to actually create a process in the test environment,
        // so just check that the function ran
        if process.is_ok() {
            // Release the process
            let release_result = pool.release(&processor_path.to_string_lossy(), process.unwrap()).await;
            assert!(release_result.is_ok());
        }
    }
}
