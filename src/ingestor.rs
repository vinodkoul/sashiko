use crate::db::Database;
use crate::events::Event;
use crate::nntp::NntpClient;
use crate::settings::{IngestionMode, Settings};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::mpsc::Sender;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

pub struct Ingestor {
    settings: Settings,
    db: Arc<Database>,
    sender: Sender<Event>,
}

impl Ingestor {
    pub fn new(settings: Settings, db: Arc<Database>, sender: Sender<Event>) -> Self {
        Self {
            settings,
            db,
            sender,
        }
    }

    pub async fn run(&self) -> Result<()> {
        match self.settings.ingestion.mode {
            IngestionMode::Nntp => self.run_nntp().await,
            IngestionMode::LocalArchive => self.run_local_archive().await,
        }
    }

    async fn run_nntp(&self) -> Result<()> {
        info!(
            "Starting NNTP Ingestor for groups: {:?}",
            self.settings.nntp.groups
        );

        loop {
            if let Err(e) = self.process_nntp_cycle().await {
                error!("NNTP Ingestion cycle failed: {}", e);
            }
            sleep(Duration::from_secs(60)).await;
        }
    }

    async fn process_nntp_cycle(&self) -> Result<()> {
        let mut client = NntpClient::connect(&self.settings.nntp.server, self.settings.nntp.port).await?;

        for group_name in &self.settings.nntp.groups {
            self.db.ensure_mailing_list(group_name, group_name).await?;

            let info = client.group(group_name).await?;
            let last_known = self.db.get_last_article_num(group_name).await?;

            info!(
                "Group {}: estimated count={}, low={}, high={}, last_known={}",
                group_name, info.number, info.low, info.high, last_known
            );

            let mut current = last_known;
            if current == 0 && info.high > 0 {
                current = info.high.saturating_sub(5);
                self.db.update_last_article_num(group_name, current).await?;
                info!("Initialized high-water mark to {}", current);
            }

            if current < info.high {
                let next_id = current + 1;
                info!("Fetching article {}", next_id);
                match client.article(&next_id.to_string()).await {
                    Ok(lines) => {
                        self.sender
                            .send(Event::ArticleFetched {
                                group: group_name.clone(),
                                article_id: next_id.to_string(),
                                content: lines,
                                raw: None,
                            })
                            .await?;
                        self.db.update_last_article_num(group_name, next_id).await?;
                        info!("Updated high-water mark to {}", next_id);
                    }
                    Err(e) => {
                        error!("Failed to fetch article {}: {}", next_id, e);
                    }
                }
            }
        }

        client.quit().await?;
        Ok(())
    }

    async fn run_local_archive(&self) -> Result<()> {
        let archive_settings = self.settings.ingestion.archive.as_ref()
            .ok_or_else(|| anyhow!("LocalArchive mode selected but no archive path provided"))?;
        
        info!("Starting Local Archive Ingestor from {:?}", archive_settings.path);

        // Get list of all blob hashes in the repo (each blob is an email)
        let output = Command::new("git")
            .arg("-c")
            .arg("safe.bareRepository=all")
            .current_dir(&archive_settings.path)
            .arg("rev-list")
            .arg("--all")
            .arg("--objects")
            .output()
            .await?;

        if !output.status.success() {
            return Err(anyhow!("Failed to list git objects: {}", String::from_utf8_lossy(&output.stderr)));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut count = 0;
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() { continue; }
            let hash = parts[0];

            // Check if it's a blob
            let cat_type = Command::new("git")
                .arg("-c")
                .arg("safe.bareRepository=all")
                .current_dir(&archive_settings.path)
                .arg("cat-file")
                .arg("-t")
                .arg(hash)
                .output()
                .await?;
            
            if !cat_type.status.success() || String::from_utf8_lossy(&cat_type.stdout).trim() != "blob" {
                continue;
            }

            // Extract the blob content
            let content_output = Command::new("git")
                .arg("-c")
                .arg("safe.bareRepository=all")
                .current_dir(&archive_settings.path)
                .arg("cat-file")
                .arg("-p")
                .arg(hash)
                .output()
                .await?;

            if content_output.status.success() {
                let raw = content_output.stdout;
                // Convert raw to lines for the 'content' field (legacy)
                let content_str = String::from_utf8_lossy(&raw);
                let lines: Vec<String> = content_str.lines().map(|s| s.to_string()).collect();

                self.sender.send(Event::ArticleFetched {
                    group: "local-archive".to_string(),
                    article_id: hash.to_string(),
                    content: lines,
                    raw: Some(raw),
                }).await?;
                
                count += 1;
                if count % 100 == 0 {
                    info!("Processed {} emails from archive", count);
                }
                
                // For testing, stop after 1000 emails
                if count >= 100 {
                    break;
                }
            } else {
                warn!("Failed to cat blob {}: {}", hash, String::from_utf8_lossy(&content_output.stderr));
            }
        }

        info!("Local archive ingestion completed. Total: {}", count);
        Ok(())
    }
}