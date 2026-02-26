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

use crate::ai::gemini::{GeminiClient, GeminiError, GenerateContentRequest};
use crate::ai::quota::QuotaManager;
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;
use tracing::error;

pub struct ProxyState {
    pub client: Arc<GeminiClient>,
    pub quota_manager: Arc<QuotaManager>,
}

pub async fn handle_generate(
    State(state): State<Arc<ProxyState>>,
    Json(request): Json<GenerateContentRequest>,
) -> impl IntoResponse {
    loop {
        // 1. Wait if globally blocked
        let _slept = state.quota_manager.wait_for_access().await;

        // 2. Try request
        match state.client.generate_content_single(&request).await {
            Ok(response) => {
                state.quota_manager.report_success().await;
                return (StatusCode::OK, Json(response)).into_response();
            }
            Err(e) => {
                if let Some(err) = e.downcast_ref::<GeminiError>() {
                    match err {
                        GeminiError::QuotaExceeded(duration) => {
                            state.quota_manager.report_quota_error(*duration).await;
                            continue;
                        }
                        GeminiError::TransientError(_, _) => {
                            state.quota_manager.report_transient_error().await;
                            continue;
                        }
                        _ => {}
                    }
                }

                error!("Gemini Proxy Error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        }
    }
}
