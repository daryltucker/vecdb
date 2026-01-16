use anyhow::{Result, Context};
use std::path::PathBuf;
use std::fs;
use crate::types::{Job, JobStatus};
use chrono::Utc;
use sysinfo::{Pid, System};

pub struct JobRegistry {
    cache_path: PathBuf,
}

impl JobRegistry {
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .context("Failed to find cache directory")?
            .join("vecdb");
        
        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)?;
        }
        
        Ok(Self {
            cache_path: cache_dir.join("active_jobs.json"),
        })
    }

    pub fn load(&self) -> Result<Vec<Job>> {
        if !self.cache_path.exists() {
            return Ok(Vec::new());
        }
        
        let content = fs::read_to_string(&self.cache_path)?;
        let jobs: Vec<Job> = serde_json::from_str(&content)?;
        
        // Filter out stale jobs where PID no longer exists
        let mut sys = System::new_all();
        sys.refresh_all();
        
        let active_jobs = jobs.into_iter().filter(|job| {
            if let JobStatus::Running = job.status {
                sys.process(Pid::from(job.pid as usize)).is_some()
            } else {
                // Keep Queued, Completed, Failed jobs for a short time?
                // For now, only track active or very recent.
                true 
            }
        }).collect();
        
        Ok(active_jobs)
    }

    pub fn save(&self, jobs: &[Job]) -> Result<()> {
        let content = serde_json::to_string_pretty(jobs)?;
        fs::write(&self.cache_path, content)?;
        Ok(())
    }

    pub fn register(&self, job_type: &str, collection: &str) -> Result<String> {
        let mut jobs = self.load()?;
        let id = uuid::Uuid::new_v4().to_string();
        
        let job = Job {
            id: id.clone(),
            job_type: job_type.to_string(),
            collection: collection.to_string(),
            status: JobStatus::Running,
            progress: 0.0,
            pid: std::process::id(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
        };
        
        jobs.push(job);
        self.save(&jobs)?;
        Ok(id)
    }

    pub fn update_progress(&self, id: &str, progress: f32) -> Result<()> {
        let mut jobs = self.load()?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.progress = progress;
            job.updated_at = Utc::now();
            self.save(&jobs)?;
        }
        Ok(())
    }

    pub fn complete(&self, id: &str) -> Result<()> {
        let mut jobs = self.load()?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Completed;
            job.progress = 1.0;
            job.updated_at = Utc::now();
            self.save(&jobs)?;
        }
        Ok(())
    }

    pub fn fail(&self, id: &str, error: &str) -> Result<()> {
        let mut jobs = self.load()?;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.status = JobStatus::Failed(error.to_string());
            job.updated_at = Utc::now();
            self.save(&jobs)?;
        }
        Ok(())
    }
}
