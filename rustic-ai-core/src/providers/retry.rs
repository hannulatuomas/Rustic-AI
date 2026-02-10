use reqwest::{RequestBuilder, Response, StatusCode};
use tokio::time::{sleep, Duration};

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: usize,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub jitter_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay_ms: 250,
            max_delay_ms: 3_000,
            jitter_ms: 100,
        }
    }
}

pub fn is_retryable_status(status: StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

pub fn is_retryable_transport_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

fn retry_delay(policy: &RetryPolicy, attempt: usize) -> Duration {
    let shift = (attempt as u32).min(12);
    let exp = 1u64 << shift;
    let base = policy.base_delay_ms.saturating_mul(exp);
    let capped = base.min(policy.max_delay_ms.max(policy.base_delay_ms));
    let jitter = if policy.jitter_ms == 0 {
        0
    } else {
        (attempt as u64 * 37) % (policy.jitter_ms + 1)
    };
    Duration::from_millis(capped.saturating_add(jitter))
}

pub async fn send_with_retry(
    builder: RequestBuilder,
    policy: &RetryPolicy,
    operation: &str,
) -> Result<Response> {
    let total_attempts = policy.max_retries.saturating_add(1);

    for attempt in 0..total_attempts {
        let Some(request) = builder.try_clone() else {
            return Err(Error::Provider(format!(
                "{operation} could not be retried because request body is not clonable"
            )));
        };

        let response = request.send().await;
        match response {
            Ok(resp) => {
                if is_retryable_status(resp.status()) && attempt + 1 < total_attempts {
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "<failed to read body>".to_owned());
                    let delay = retry_delay(policy, attempt);
                    tracing::warn!(
                        operation,
                        status = %status,
                        attempt,
                        total_attempts,
                        delay_ms = delay.as_millis(),
                        "retrying provider request after retryable HTTP status"
                    );
                    tracing::debug!(operation, %status, body, "retryable HTTP response body");
                    sleep(delay).await;
                    continue;
                }

                return Ok(resp);
            }
            Err(err) => {
                if is_retryable_transport_error(&err) && attempt + 1 < total_attempts {
                    let delay = retry_delay(policy, attempt);
                    tracing::warn!(
                        operation,
                        error = %err,
                        attempt,
                        total_attempts,
                        delay_ms = delay.as_millis(),
                        "retrying provider request after retryable transport error"
                    );
                    sleep(delay).await;
                    continue;
                }

                return Err(Error::Provider(format!(
                    "{operation} failed{}: {err}",
                    if is_retryable_transport_error(&err) {
                        " after retries"
                    } else {
                        ""
                    }
                )));
            }
        }
    }

    Err(Error::Provider(format!(
        "{operation} failed after retry budget was exhausted"
    )))
}
