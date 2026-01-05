#[cfg(test)]
mod tests {
    use crate::agent::{Agent, prompts::PromptRegistry, tools::ToolBox};
    use crate::ai::gemini::GeminiClient;
    use serde_json::json;
    use std::path::PathBuf;

    fn get_test_paths() -> (PathBuf, PathBuf) {
        let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let linux_path = root.join("linux");
        let prompts_path = root.join("review-prompts");
        (linux_path, prompts_path)
    }

    #[tokio::test]
    async fn test_agent_integration_sanity() {
        let _ = tracing_subscriber::fmt::try_init();
        // Skip if no API key
        if std::env::var("LLM_API_KEY").is_err() {
            println!("Skipping AI integration test: LLM_API_KEY not set");
            return;
        }

        let (linux_path, prompts_path) = get_test_paths();
        
        // Setup dependencies
        // Use flash model for tests to save cost/latency
        let client = GeminiClient::new("gemini-3-flash-preview".to_string());
        let tools = ToolBox::new(linux_path, prompts_path);
        let prompts = PromptRegistry::new(PathBuf::from("review-prompts"));

        let mut agent = Agent::new(client, tools, prompts);

        // Create a dummy patchset that invites checking a file
        // We hope the model decides to check README or similar.
        // Even if it doesn't, we just want to ensure the loop runs and returns a result without crashing.
        let patchset = json!({
            "subject": "Documentation: Fix typo in README",
            "author": "Test User <test@example.com>",
            "patches": [
                {
                    "index": 1,
                    "diff": "diff --git a/README b/README\nindex 1234567..89abcdef 100644\n--- a/README\n+++ b/README\n@@ -1,1 +1,1 @@\n-Linux kernel\n+The Linux kernel\n"
                }
            ]
        });

        let result = agent.run(patchset).await;
        
        match result {
            Ok(review) => {
                assert!(!review.is_empty(), "Review should not be empty");
                println!("Agent review output: {}", review);
            }
            Err(e) => {
                if e.to_string().contains("Agent exceeded maximum turns") {
                     println!("Agent reached max turns, which confirms it was running and using tools. Success.");
                } else {
                     panic!("Agent run failed: {}", e);
                }
            }
        }
    }
}
