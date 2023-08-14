use tracing::{error, info};

use super::*;
#[derive(clap::Args)]
pub struct Args {
    #[arg(long, short, value_parser = VideoDevice::new_checked)]
    video_device: VideoDevice,
    #[arg(long, short)]
    output_path: PathBuf,
}
#[derive(Debug)]
pub struct GStreamerReaderDumper {
    pub process: AbortOnDrop<()>,
}

impl GStreamerReaderDumper {
    #[tracing::instrument]
    pub fn new(
        Args {
            video_device,
            output_path,
        }: Args,
    ) -> Result<Self> {
        tracing::info!("spawning gstreamer process");
        let process = tokio::task::spawn_blocking(|| match gstreamer_process::start_stream() {
            Ok(_) => info!("process has finished"),
            Err(message) => error!(?message, "bye bye"),
        })
        .abort_on_drop();
        Ok(Self { process })
    }

    pub async fn wait_for_finish(self) -> Result<()> {
        self.process
            .await
            .map_err(eyre::Report::from)
            .map_err(|v| v.wrap_err("waiting for gstreamer process to finish"))
    }
}
