use tracing_subscriber::EnvFilter;

pub fn init_tracing(default_level: &str) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}
