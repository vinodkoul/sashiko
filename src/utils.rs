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

use regex::Regex;
use std::sync::OnceLock;

static KEY_REGEX: OnceLock<Regex> = OnceLock::new();
static URL_CRED_REGEX: OnceLock<Regex> = OnceLock::new();

/// Redacts sensitive information from a string.
///
/// Specifically targets:
/// - API keys in query parameters (e.g., `key=AIza...`)
/// - Credentials in URLs (e.g., `https://user:pass@host`)
pub fn redact_secret(s: &str) -> String {
    let key_re =
        KEY_REGEX.get_or_init(|| Regex::new(r"(?i)(key|token|secret)=([a-zA-Z0-9_\-]+)").unwrap());

    let url_cred_re = URL_CRED_REGEX.get_or_init(|| Regex::new(r"://([^/:]+):([^/@]+)@").unwrap());

    let redacted_params = key_re.replace_all(s, "$1=[REDACTED]");
    let redacted_url = url_cred_re.replace_all(&redacted_params, "://[REDACTED]:[REDACTED]@");

    redacted_url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_gemini_key() {
        let url = "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-pro:generateContent?key=AIzaSyD-12345";
        let redacted = redact_secret(url);
        assert_eq!(
            redacted,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-pro:generateContent?key=[REDACTED]"
        );
    }

    #[test]
    fn test_redact_git_credentials() {
        let url = "https://user:password123@github.com/torvalds/linux.git";
        let redacted = redact_secret(url);
        assert_eq!(
            redacted,
            "https://[REDACTED]:[REDACTED]@github.com/torvalds/linux.git"
        );
    }

    #[test]
    fn test_redact_mixed() {
        let s = "Error connecting to https://user:pass@host/api?key=secret_value";
        let redacted = redact_secret(s);
        assert_eq!(
            redacted,
            "Error connecting to https://[REDACTED]:[REDACTED]@host/api?key=[REDACTED]"
        );
    }

    #[test]
    fn test_no_secrets() {
        let s = "https://github.com/torvalds/linux.git";
        let redacted = redact_secret(s);
        assert_eq!(redacted, s);
    }
}
