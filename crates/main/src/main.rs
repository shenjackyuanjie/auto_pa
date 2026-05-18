use tracing::event;

use crate::logger::LoggerConfig;

pub mod logger;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logger::init(LoggerConfig::default());
    for device in hdc_api::device::get_devices().await? {
        event!(tracing::Level::INFO, "Device: {}", device.format_tag());
        device.dump_layout_to_file("dump.json").await?;
    }

    Ok(())
}
