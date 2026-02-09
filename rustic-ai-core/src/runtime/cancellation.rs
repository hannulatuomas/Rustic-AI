use tokio_util::sync::CancellationToken;

pub fn new_cancellation_token() -> CancellationToken {
    CancellationToken::new()
}
