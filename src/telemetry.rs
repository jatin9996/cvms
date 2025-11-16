use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

pub fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_else(|_| EnvFilter::new(Level::INFO.as_str()));

    let fmt_layer = fmt::layer().with_target(false).with_level(true);
    let subscriber = Registry::default().with(env_filter).with(fmt_layer);
    let _ = tracing::subscriber::set_global_default(subscriber);
}
