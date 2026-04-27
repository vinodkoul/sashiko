// Copyright 2026 The Sashiko Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::Client;
use sashiko::api::{PatchsetsResponse, SubmitRequest, SubmitResponse};
use sashiko::settings::Settings;
use serde_json::{Value, from_str};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

#[derive(Parser)]
#[command(name = "sashiko-cli")]
#[command(about = "CLI tool for interacting with Sashiko", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Override server URL (default: from settings or http://127.0.0.1:8080)
    #[arg(long, global = true, env = "SASHIKO_SERVER")]
    server: Option<String>,

    /// Output format (text, json)
    #[arg(long, global = true, default_value = "text")]
    format: OutputFormat,
}

#[derive(Clone, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Submit a patch or range for review
    Submit {
        /// Revision range, commit SHA, or path to mbox file.
        /// Defaults to "HEAD" if in a git repo, or reads from stdin if piped.
        #[arg(value_name = "INPUT")]
        input: Option<String>,

        /// Explicitly set type (overrides auto-detection)
        #[arg(long, value_enum)]
        r#type: Option<SubmitType>,

        /// Override repository path (defaults to settings)
        #[arg(long, short = 'r')]
        repo: Option<PathBuf>,

        /// Baseline commit (for mbox injection only)
        #[arg(long)]
        baseline: Option<String>,

        /// Skip review for patches matching subject pattern (with wildcards, e.g. mm:*)
        #[arg(long, value_name = "PATTERN")]
        skip_subject: Option<Vec<String>>,

        /// Only review patches matching subject pattern (with wildcards, e.g. *PRODKERNEL*)
        #[arg(long, value_name = "PATTERN")]
        only_subject: Option<Vec<String>>,
    },
    /// Show server status and statistics
    Status,
    /// List patchsets or reviews
    List {
        /// Filter query (e.g. "pending", "failed", "linux-mm")
        #[arg(value_name = "FILTER")]
        filter: Option<String>,

        /// Page number
        #[arg(long, default_value_t = 1)]
        page: usize,

        /// Items per page
        #[arg(long, default_value_t = 20)]
        per_page: usize,
    },
    /// Show details of a patchset or review
    Show {
        /// ID of the patchset or "latest"
        #[arg(default_value = "latest")]
        id: String,
    },
    /// Cancel a pending review
    Cancel {
        /// ID of the patchset to cancel
        id: i64,

        /// Force cancel even if the review is already in progress
        #[arg(long, short)]
        force: bool,
    },
    /// Run a local review (or queue to running server)
    Local {
        /// Git revision, range (e.g. HEAD~3..HEAD), or commit SHA
        #[arg(default_value = "HEAD")]
        input: String,

        /// Baseline reference (default: parent of first commit in range)
        #[arg(long)]
        baseline: Option<String>,

        /// Path to git repository (default: current directory)
        #[arg(long, short = 'r')]
        repo: Option<PathBuf>,

        /// Skip AI review, only test patch application
        #[arg(long)]
        no_ai: bool,

        /// Custom prompt to append to the review task
        #[arg(long)]
        custom_prompt: Option<String>,

        /// Force local execution even if server is running
        #[arg(long)]
        force_local: bool,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum SubmitType {
    /// Submit a raw mbox file (or - for stdin)
    Mbox,
    /// Submit a single remote commit
    Remote,
    /// Submit a range of remote commits
    Range,
    /// Fetch a thread from lore.kernel.org by message ID
    Thread,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load settings, falling back to defaults if file missing/invalid
    let base_url = cli.server.unwrap_or_else(|| {
        Settings::new()
            .map(|s| {
                if s.server.host.contains(':') {
                    format!("http://[::1]:{}", s.server.port)
                } else {
                    format!("http://{}:{}", s.server.host, s.server.port)
                }
            })
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
    });

    let client = Client::new();

    if let Err(e) = run_command(cli.command, &client, &base_url, cli.format).await {
        print_colored(Color::Red, "Error: ");
        println!("{}", e);

        // Provide helpful hints for common errors
        if let Some(req_err) = e.downcast_ref::<reqwest::Error>() {
            if req_err.is_connect() {
                println!("\nHint: Is the Sashiko server running at {}?", base_url);
                println!("      You can start it with `cargo run --bin sashiko`");
            } else if let Some(status) = req_err.status() {
                if status == reqwest::StatusCode::NOT_FOUND {
                    println!("\nHint: The requested resource was not found.");
                } else if status == reqwest::StatusCode::BAD_REQUEST {
                    println!("\nHint: The request was invalid. Check your arguments.");
                }
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

async fn run_command(
    command: Commands,
    client: &Client,
    base_url: &str,
    format: OutputFormat,
) -> Result<()> {
    match command {
        Commands::Submit {
            input,
            r#type,
            repo,
            baseline,
            skip_subject,
            only_subject,
        } => {
            handle_submit(
                client,
                base_url,
                input,
                r#type,
                repo,
                baseline,
                skip_subject,
                only_subject,
                format,
            )
            .await
        }
        Commands::Status => handle_status(client, base_url, format).await,
        Commands::List {
            filter,
            page,
            per_page,
        } => handle_list(client, base_url, page, per_page, filter, format).await,
        Commands::Show { id } => handle_show(client, base_url, id, format).await,
        Commands::Cancel { id, force } => handle_cancel(client, base_url, id, force, format).await,
        Commands::Local {
            input,
            baseline,
            repo,
            no_ai,
            custom_prompt,
            force_local,
        } => {
            handle_local(
                client,
                base_url,
                input,
                baseline,
                repo,
                no_ai,
                custom_prompt,
                force_local,
                format,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_submit(
    client: &Client,
    base_url: &str,
    input: Option<String>,
    explicit_type: Option<SubmitType>,
    repo: Option<PathBuf>,
    baseline: Option<String>,
    skip_subjects: Option<Vec<String>>,
    only_subjects: Option<Vec<String>>,
    format: OutputFormat,
) -> Result<()> {
    let url = format!("{}/api/submit", base_url);

    // DWIM Detection Logic
    let (submission_type, target) = if let Some(t) = explicit_type {
        (t, input.unwrap_or_else(|| "HEAD".to_string()))
    } else {
        // Auto-detect based on input
        if let Some(s) = input {
            if s == "-" {
                (SubmitType::Mbox, s)
            } else if s.contains("..") {
                (SubmitType::Range, s)
            } else if s.contains('@') && !s.contains('/') && !s.contains('\\') {
                // If it looks like an email address/msgid and doesn't look like a path, assume Thread
                (SubmitType::Thread, s)
            } else if PathBuf::from(&s).exists() {
                // If it's a file, assume mbox. If it's a dir, maybe repo?
                // For safety, if it looks like a commit (hex), prefer Remote unless file exists.
                // But filenames can look like anything.
                // Sashiko deals with mbox files primarily.
                let p = PathBuf::from(&s);
                if p.is_file() {
                    (SubmitType::Mbox, s)
                } else {
                    // Not a file, assume commit ref
                    (SubmitType::Remote, s)
                }
            } else {
                // Not a file on disk (or we can't see it). Assume commit ref.
                (SubmitType::Remote, s)
            }
        } else {
            // No input provided.
            // Check if stdin is piped
            if !std::io::stdin().is_terminal() {
                (SubmitType::Mbox, "-".to_string())
            } else {
                // Default to HEAD
                (SubmitType::Remote, "HEAD".to_string())
            }
        }
    };

    let payload = match submission_type {
        SubmitType::Mbox => {
            let content = if target == "-" {
                let mut buffer = String::new();
                std::io::stdin()
                    .read_to_string(&mut buffer)
                    .context("Failed to read from stdin")?;
                buffer
            } else {
                std::fs::read_to_string(&target)
                    .with_context(|| format!("Failed to read file {:?}", target))?
            };
            SubmitRequest::Inject {
                raw: content,
                base_commit: baseline,
                skip_subjects: skip_subjects.clone(),
                only_subjects: only_subjects.clone(),
            }
        }
        SubmitType::Remote => {
            let repo_path = repo.map(|p| p.to_string_lossy().to_string());

            SubmitRequest::Remote {
                sha: target,
                repo: repo_path,
                skip_subjects: skip_subjects.clone(),
                only_subjects: only_subjects.clone(),
            }
        }
        SubmitType::Range => {
            let repo_path = repo.map(|p| p.to_string_lossy().to_string());

            SubmitRequest::RemoteRange {
                sha: target,
                repo: repo_path,
                skip_subjects: skip_subjects.clone(),
                only_subjects: only_subjects.clone(),
            }
        }
        SubmitType::Thread => SubmitRequest::Thread { msgid: target },
    };

    let resp = client.post(&url).json(&payload).send().await?;

    if resp.status().is_success() {
        let result: SubmitResponse = resp.json().await?;
        match format {
            OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
            OutputFormat::Text => {
                print_colored(Color::Green, "Success: ");
                println!("Submission accepted. ID: {}", result.id);
            }
        }
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Submission failed ({}): {}", status, text));
    }

    Ok(())
}

async fn handle_status(client: &Client, base_url: &str, format: OutputFormat) -> Result<()> {
    let url = format!("{}/api/stats", base_url);
    let resp = client.get(&url).send().await?;

    if resp.status().is_success() {
        let stats: Value = resp.json().await?;

        match format {
            OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&stats)?),
            OutputFormat::Text => {
                print_colored(Color::Cyan, "Server Status:\n");
                println!(
                    "  Version:   {}",
                    stats["version"].as_str().unwrap_or("unknown")
                );
                println!("  Messages:  {}", stats["messages"]);
                println!("  Patchsets: {}", stats["patchsets"]);

                if let Some(breakdown) = stats.get("breakdown") {
                    println!("\nQueue Breakdown:");
                    let items = [
                        ("Pending", "pending"),
                        ("In Review", "reviewing"),
                        ("Reviewed", "reviewed"),
                        ("Failed", "failed"),
                        ("Apply Failed", "failed_to_apply"),
                        ("Incomplete", "incomplete"),
                    ];

                    let zero = serde_json::json!(0);
                    for (label, key) in items {
                        let val = breakdown.get(key).unwrap_or(&zero);
                        println!("  {:<15} {}", label, val);
                    }
                }
            }
        }
    } else {
        return Err(anyhow::anyhow!("Failed to get status: {}", resp.status()));
    }

    Ok(())
}

async fn handle_list(
    client: &Client,
    base_url: &str,
    page: usize,
    per_page: usize,
    filter: Option<String>,
    format: OutputFormat,
) -> Result<()> {
    let mut url = format!(
        "{}/api/patchsets?page={}&per_page={}",
        base_url, page, per_page
    );
    if let Some(q) = filter {
        url.push_str(&format!("&q={}", q));
    }

    let resp = client.get(&url).send().await?;

    if resp.status().is_success() {
        let data: PatchsetsResponse = resp.json().await?;

        match format {
            OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&data)?),
            OutputFormat::Text => {
                if data.items.is_empty() {
                    println!("No items found.");
                    return Ok(());
                }

                println!(
                    "{:<10} {:<18} {:<50} {:<20}",
                    "ID", "Status", "Subject", "Date"
                );
                println!("{:-<10} {:-<18} {:-<50} {:-<20}", "", "", "", "");

                for item in data.items {
                    let status_str = item.status.as_deref().unwrap_or("Unknown");

                    let status_color = match status_str {
                        "Reviewed" => Color::Green,
                        "Embargoed" => Color::Magenta,
                        "Failed" | "Error" | "Failed To Apply" => Color::Red,
                        "Pending" | "In Review" => Color::Yellow,
                        "Cancelled" => Color::Red,
                        _ => Color::White,
                    };

                    print!("{:<10} ", item.id);
                    print_colored(status_color, &format!("{:<18}", status_str));

                    let subject = item.subject.unwrap_or_else(|| "(no subject)".to_string());
                    let subject_display = if subject.len() > 48 {
                        format!("{}...", &subject[..45])
                    } else {
                        subject
                    };

                    let date_display = if let Some(ts) = item.date {
                        format_timestamp(ts)
                    } else {
                        "-".to_string()
                    };

                    println!(" {:<50} {}", subject_display, date_display);
                }

                println!(
                    "\nPage {} of {} (Total: {})",
                    data.page,
                    data.total.div_ceil(data.per_page),
                    data.total
                );
            }
        }
    } else {
        return Err(anyhow::anyhow!(
            "Failed to list patchsets: {}",
            resp.status()
        ));
    }

    Ok(())
}

async fn handle_show(
    client: &Client,
    base_url: &str,
    mut id: String,
    format: OutputFormat,
) -> Result<()> {
    if id == "latest" {
        // Fetch list to find latest
        let list_url = format!("{}/api/patchsets?page=1&per_page=1", base_url);
        let resp = client.get(&list_url).send().await?;
        if resp.status().is_success() {
            let data: PatchsetsResponse = resp.json().await?;
            if let Some(latest) = data.items.first() {
                id = latest.id.to_string();
            } else {
                return Err(anyhow::anyhow!("No patchsets found"));
            }
        } else {
            return Err(anyhow::anyhow!(
                "Failed to find latest patchset: {}",
                resp.status()
            ));
        }
    }

    let url = format!("{}/api/patch?id={}", base_url, id);
    let resp = client.get(&url).send().await?;

    if resp.status().is_success() {
        let mut details: Value = resp.json().await?;
        let status = details["status"].as_str().unwrap_or("").to_string();

        // Extract the actual numeric ID for subsequent calls
        let numeric_id = details["id"].to_string();

        // Fetch review if available
        let mut review_data = None;
        if status == "Reviewed" || status == "Failed" || status == "Failed To Apply" {
            let review_url = format!("{}/api/review_log?patchset_id={}", base_url, numeric_id);
            let review_resp = client.get(&review_url).send().await?;

            if review_resp.status().is_success() {
                review_data = Some(review_resp.json::<Value>().await?);
            }
        }

        match format {
            OutputFormat::Json => {
                if let Some(r) = review_data {
                    details["review"] = r;
                }
                println!("{}", serde_json::to_string_pretty(&details)?);
            }
            OutputFormat::Text => {
                print_colored(Color::Cyan, "Patchset Details:\n");
                println!("  ID:        {}", details["id"]);
                println!("  Subject:   {}", details["subject"].as_str().unwrap_or(""));
                println!("  Author:    {}", details["author"].as_str().unwrap_or(""));
                let status_str = details["status"].as_str().unwrap_or("");
                if status_str == "Embargoed" {
                    if let Some(until_ts) = details.get("embargo_until").and_then(|u| u.as_i64()) {
                        println!(
                            "  Status:    Embargoed until {}",
                            format_timestamp(until_ts)
                        );
                    } else {
                        println!("  Status:    Embargoed");
                    }
                } else {
                    println!("  Status:    {}", status_str);
                }

                if let Some(ts) = details["date"].as_i64() {
                    println!("  Date:      {}", format_timestamp(ts));
                }

                if let Some(reason) = details.get("failed_reason").and_then(|r| r.as_str()) {
                    print_colored(Color::Red, "\nFailure Reason: ");
                    println!("{}", reason);
                }

                if let Some(patches) = details.get("patches").and_then(|p| p.as_array()) {
                    println!("\nPatches ({}):", patches.len());
                    for patch in patches {
                        let idx = patch["part_index"].as_i64().unwrap_or(0);
                        let status = patch["status"].as_str().unwrap_or("");
                        let apply_err = patch["apply_error"].as_str();
                        let p_id = patch["id"].as_i64().unwrap_or(0);

                        let mut patch_review_status = None;
                        let mut has_issues = false;
                        if let Some(reviews) = details.get("reviews").and_then(|r| r.as_array()) {
                            for r in reviews {
                                if r.get("patch_id").and_then(|id| id.as_i64()) == Some(p_id) {
                                    patch_review_status = r.get("status").and_then(|s| s.as_str());
                                    if let Some(inline) =
                                        r.get("inline_review").and_then(|s| s.as_str())
                                        && inline != "No issues found."
                                        && !inline.is_empty()
                                    {
                                        has_issues = true;
                                    }
                                }
                            }
                        }

                        print!("  [{}] {}", idx, patch["subject"].as_str().unwrap_or(""));
                        if !status.is_empty() && status != "Pending" {
                            print!(" (");
                            let color = match status {
                                "Failed" | "Failed To Apply" | "Error" => Color::Red,
                                "Embargoed" => Color::Magenta,
                                _ => Color::Green,
                            };
                            print_colored(color, status);
                            print!(")");
                        }

                        if let Some(rev_status) = patch_review_status {
                            print!(" [");
                            let color = if has_issues {
                                Color::Yellow
                            } else if rev_status == "Failed" {
                                Color::Red
                            } else {
                                Color::Green
                            };
                            let label = if has_issues {
                                "Issues Found"
                            } else {
                                rev_status
                            };
                            print_colored(color, label);
                            print!("]");
                        }
                        println!();

                        if let Some(err) = apply_err {
                            print_colored(Color::Red, "      Error: ");
                            println!("{}", err.trim());
                        }
                    }
                }

                if let Some(review) = review_data {
                    println!("\nReview Summary:");
                    if let Some(verdict) = review.get("verdict").and_then(|v| v.as_str()) {
                        let color = match verdict {
                            "LGTM" => Color::Green,
                            "Request Changes" => Color::Red,
                            _ => Color::Yellow,
                        };
                        print!("  Verdict: ");
                        print_colored(color, verdict);
                        println!();
                    }

                    if let Some(model) = review.get("model").and_then(|m| m.as_str()) {
                        println!("  Model:   {}", model);
                    }

                    if let Some(summary) = review.get("summary").and_then(|s| s.as_str())
                        && summary != "No summary available."
                        && !summary.is_empty()
                    {
                        println!("\n{}", summary.trim());
                    }

                    if let Some(patches) = details.get("patches").and_then(|p| p.as_array()) {
                        println!();
                        for patch in patches {
                            let idx = patch["part_index"].as_i64().unwrap_or(0);
                            let subject = patch["subject"].as_str().unwrap_or("");
                            let p_id = patch["id"].as_i64().unwrap_or(0);

                            let mut patch_review = None;
                            if let Some(reviews) = details.get("reviews").and_then(|r| r.as_array())
                            {
                                for r in reviews {
                                    if r.get("patch_id").and_then(|id| id.as_i64()) == Some(p_id) {
                                        let status = r.get("status").and_then(|s| s.as_str());
                                        let current_status =
                                            patch_review.and_then(|pr: &serde_json::Value| {
                                                pr.get("status").and_then(|s| s.as_str())
                                            });
                                        if status == Some("Reviewed")
                                            || current_status != Some("Reviewed")
                                        {
                                            patch_review = Some(r);
                                        }
                                    }
                                }
                            }

                            if let Some(r) = patch_review
                                && let Some(output_str) = r.get("output").and_then(|o| o.as_str())
                                && let Ok(output_json) = from_str::<Value>(output_str)
                                && let Some(findings) =
                                    output_json.get("findings").and_then(|f| f.as_array())
                            {
                                let inline = r.get("inline_review").and_then(|s| s.as_str());
                                print_findings_summary(
                                    &format!("Patch {}: {}", idx, subject),
                                    findings,
                                    inline,
                                );
                            }
                        }
                    }
                } else if let Some(logs) = details.get("baseline_logs").and_then(|l| l.as_str()) {
                    // Fallback to baseline logs if review is missing (e.g. Failed To Apply during baseline prep)
                    if status == "Failed To Apply" {
                        println!("\nBaseline Logs:\n{}", logs);
                    }
                }
            }
        }
    } else {
        return Err(anyhow::anyhow!(
            "Failed to show patchset: {}",
            resp.status()
        ));
    }

    Ok(())
}

async fn handle_cancel(
    client: &Client,
    base_url: &str,
    id: i64,
    force: bool,
    format: OutputFormat,
) -> Result<()> {
    let url = format!("{}/api/patchset/cancel?id={}&force={}", base_url, id, force);
    let resp = client.post(&url).send().await?;

    if resp.status().is_success() {
        let result: serde_json::Value = resp.json().await?;
        match format {
            OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
            OutputFormat::Text => {
                let status = result["status"].as_str().unwrap_or("");
                if status == "cancelled" {
                    print_colored(Color::Green, "Cancelled: ");
                    println!("Patchset {} has been cancelled.", id);
                } else {
                    print_colored(Color::Yellow, "Not modified: ");
                    println!(
                        "{}",
                        result["reason"]
                            .as_str()
                            .unwrap_or("Patchset could not be cancelled.")
                    );
                }
            }
        }
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Cancel failed ({}): {}", status, text));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_local(
    client: &Client,
    base_url: &str,
    input: String,
    baseline: Option<String>,
    repo: Option<PathBuf>,
    no_ai: bool,
    custom_prompt: Option<String>,
    force_local: bool,
    format: OutputFormat,
) -> Result<()> {
    // Determine repository path
    let repo_path = if let Some(r) = repo {
        r
    } else {
        let cwd = std::env::current_dir().context("Failed to get current directory")?;
        // Verify CWD is a git repo
        if !cwd.join(".git").exists() {
            return Err(anyhow::anyhow!(
                "Current directory is not a git repository. Use --repo to specify one."
            ));
        }
        cwd
    };

    // Check if server is running (unless --force-local)
    if !force_local && let Ok(settings) = Settings::new() {
        let addr = format!("{}:{}", settings.server.host, settings.server.port);
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            // Server is running — submit via API
            let submit_type = if input.contains("..") {
                SubmitType::Range
            } else {
                SubmitType::Remote
            };
            let repo_str = Some(repo_path.to_string_lossy().to_string());

            let url = format!("{}/api/submit", base_url);
            let payload = match submit_type {
                SubmitType::Range => SubmitRequest::RemoteRange {
                    sha: input.clone(),
                    repo: repo_str,
                    skip_subjects: None,
                    only_subjects: None,
                },
                _ => SubmitRequest::Remote {
                    sha: input.clone(),
                    repo: repo_str,
                    skip_subjects: None,
                    only_subjects: None,
                },
            };

            let resp = client.post(&url).json(&payload).send().await?;
            if resp.status().is_success() {
                let result: SubmitResponse = resp.json().await?;
                print_colored(Color::Green, "Queued: ");
                println!(
                    "Review submitted (ID: {}). View at {}/",
                    result.id, base_url
                );
                println!("\nUse `sashiko-cli show {}` to check progress.", result.id);
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow::anyhow!(
                    "Failed to queue review ({}): {}",
                    status,
                    text
                ));
            }
            return Ok(());
        }
    }
    // Cold start path: run review locally via sashiko-review subprocess
    eprint_phase(1, 4, &format!("Extracting patches from {}...", input));

    // Resolve commits
    let shas = if input.contains("..") {
        sashiko::git_ops::resolve_git_range(&repo_path, &input).await?
    } else {
        // Single ref — resolve to SHA
        let sha = sashiko::git_ops::get_commit_hash(&repo_path, &input).await?;
        vec![sha]
    };

    eprintln!(
        " ({} commit{})",
        shas.len(),
        if shas.len() == 1 { "" } else { "s" }
    );

    // Extract patch metadata and build ReviewInput
    let mut patches = Vec::new();
    for (i, sha) in shas.iter().enumerate() {
        let meta = sashiko::git_ops::extract_patch_metadata(&repo_path, sha)
            .await
            .with_context(|| format!("Failed to extract metadata for commit {}", sha))?;
        patches.push(sashiko::worker::PatchInput {
            index: (i + 1) as i64,
            diff: meta.diff,
            subject: Some(meta.subject),
            author: Some(meta.author),
            date: Some(meta.timestamp),
            message_id: None,
            commit_id: Some(sha.clone()),
        });
    }

    let review_input = sashiko::worker::ReviewInput {
        id: 0, // Local review, no DB ID
        subject: patches
            .first()
            .and_then(|p| p.subject.clone())
            .unwrap_or_else(|| input.clone()),
        patches,
    };

    let review_json =
        serde_json::to_string(&review_input).context("Failed to serialize review input")?;

    // Locate sashiko-review binary
    let review_bin = find_review_binary()?;

    // Build subprocess args
    let baseline_ref = if let Some(b) = &baseline {
        b.clone()
    } else {
        // Default: parent of first commit
        let first_sha = &shas[0];
        format!("{}^", first_sha)
    };

    let mut args = vec!["--baseline".to_string(), baseline_ref];

    if no_ai {
        args.push("--no-ai".to_string());
    }
    if let Some(prompt) = &custom_prompt {
        args.push("--custom-prompt".to_string());
        args.push(prompt.clone());
    }

    eprint_phase(2, 4, "Starting review subprocess...");
    eprintln!();

    // Spawn review subprocess
    let mut child = tokio::process::Command::new(&review_bin)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("SASHIKO_LOG_PLAIN", "1")
        .spawn()
        .with_context(|| format!("Failed to start review binary: {:?}", review_bin))?;

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(format!("{}\n", review_json).as_bytes())
            .await
            .context("Failed to write to review subprocess stdin")?;
        drop(stdin);
    }

    // Stream stderr for progress in a background task
    let stderr = child.stderr.take();
    let stderr_handle = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut saw_applying = false;
            let mut saw_ai_review = false;
            while let Ok(Some(line)) = lines.next_line().await {
                if !saw_applying && line.contains("Applying") {
                    saw_applying = true;
                    eprint_phase(2, 4, "Applying patches to worktree...");
                    eprintln!();
                }
                if !saw_ai_review && line.contains("Starting AI review") {
                    saw_ai_review = true;
                    eprint_phase(3, 4, "AI review in progress...");
                    eprintln!();
                }
                if line.contains("AI review completed") {
                    eprint_phase(4, 4, "Review complete.");
                    eprintln!();
                }
            }
        }
    });

    // Capture stdout
    let output = child
        .wait_with_output()
        .await
        .context("Failed to wait for review subprocess")?;

    let _ = stderr_handle.await;

    let exit_code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if stdout.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "Review subprocess produced no output (exit code: {})",
            exit_code
        ));
    }

    // Parse the review output
    let result: Value =
        serde_json::from_str(stdout.trim()).context("Failed to parse review output JSON")?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text => {
            print_local_review_results(&result, &input);
        }
    }

    // Determine exit code based on findings
    if let Some(err) = result.get("error").and_then(|e| e.as_str())
        && !err.is_empty()
    {
        std::process::exit(3);
    }
    if let Some(review) = result.get("review")
        && let Some(findings) = review.get("findings").and_then(|f| f.as_array())
    {
        let (c, h, _, _) = count_severities(findings);
        if c > 0 || h > 0 {
            std::process::exit(1);
        }
    }

    Ok(())
}

fn eprint_phase(current: usize, total: usize, msg: &str) {
    eprint!("[{}/{}] {}", current, total, msg);
}

fn find_review_binary() -> Result<PathBuf> {
    // Try same directory as current executable
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let candidate = dir.join("sashiko-review");
        if candidate.exists() {
            return Ok(candidate);
        }
        // Also check for "review" (cargo build output name)
        let candidate = dir.join("review");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Try PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("sashiko-review")
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok(PathBuf::from(path));
    }

    Err(anyhow::anyhow!(
        "Cannot find sashiko-review binary.\n\
         Build it with: cargo build --bin review\n\
         Or specify its location in PATH."
    ))
}

fn print_local_review_results(result: &Value, input: &str) {
    let baseline = result["baseline"].as_str().unwrap_or("unknown");
    print_colored(Color::Cyan, "Local Review Results\n");
    println!("  Input:    {}", input);
    println!("  Baseline: {}", baseline);

    // Show patch application status
    if let Some(patches) = result.get("patches").and_then(|p| p.as_array()) {
        println!("\nPatch Application:");
        for p in patches {
            let idx = p["index"].as_i64().unwrap_or(0);
            let status = p["status"].as_str().unwrap_or("unknown");
            let method = p["method"].as_str().unwrap_or("");
            let color = match status {
                "applied" => Color::Green,
                "failed" | "error" => Color::Red,
                _ => Color::Yellow,
            };
            print!("  [{}] ", idx);
            print_colored(color, status);
            if !method.is_empty() {
                print!(" ({})", method);
            }
            println!();
            if let Some(err) = p.get("error").and_then(|e| e.as_str()) {
                print_colored(Color::Red, "      Error: ");
                println!("{}", err);
            }
        }
    }

    // Show error if present
    if let Some(err) = result.get("error").and_then(|e| e.as_str())
        && !err.is_empty()
    {
        println!();
        print_colored(Color::Red, "Error: ");
        println!("{}", err);
        return;
    }

    // Show review results
    if let Some(review) = result.get("review")
        && let Some(findings) = review.get("findings").and_then(|f| f.as_array())
    {
        let inline = result.get("inline_review").and_then(|s| s.as_str());
        println!();
        if !print_findings_summary("Review Findings:", findings, inline) {
            print_colored(Color::Green, "\nNo issues found.\n");
        }
    }

    // Show token usage
    let tokens_in = result["tokens_in"].as_u64().unwrap_or(0);
    let tokens_out = result["tokens_out"].as_u64().unwrap_or(0);
    let tokens_cached = result["tokens_cached"].as_u64().unwrap_or(0);
    if tokens_in > 0 || tokens_out > 0 {
        println!(
            "\nTokens: {} in / {} out / {} cached",
            tokens_in, tokens_out, tokens_cached
        );
    }
}

/// Count finding severities from a findings JSON array.
fn count_severities(findings: &[Value]) -> (usize, usize, usize, usize) {
    let mut counts = std::collections::HashMap::new();
    for f in findings {
        if let Some(sev) = f.get("severity").and_then(|s| s.as_str()) {
            *counts.entry(sev.to_lowercase()).or_insert(0) += 1;
        }
    }
    (
        counts.get("critical").copied().unwrap_or(0),
        counts.get("high").copied().unwrap_or(0),
        counts.get("medium").copied().unwrap_or(0),
        counts.get("low").copied().unwrap_or(0),
    )
}

/// Print a findings summary line with severity counts if any findings exist.
/// Returns true if findings were printed.
fn print_findings_summary(label: &str, findings: &[Value], inline_review: Option<&str>) -> bool {
    let (c, h, m, l) = count_severities(findings);
    if c == 0 && h == 0 && m == 0 && l == 0 {
        return false;
    }
    println!("{}", label);
    println!(
        "Critical: {} · High: {} · Medium: {} · Low: {}\n",
        c, h, m, l
    );
    if let Some(inline) = inline_review
        && !inline.is_empty()
        && inline != "No issues found."
    {
        println!("{}", inline.trim());
    }
    println!();
    true
}

fn print_colored(color: Color, text: &str) {
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);
    stdout
        .set_color(ColorSpec::new().set_fg(Some(color)))
        .unwrap();
    write!(&mut stdout, "{}", text).unwrap();
    stdout.reset().unwrap();
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return "-".to_string();
    }
    match Utc.timestamp_opt(ts, 0) {
        chrono::LocalResult::Single(dt) => {
            let local_dt: DateTime<Local> = DateTime::from(dt);
            local_dt.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        _ => ts.to_string(),
    }
}
