use tokio_util::sync::CancellationToken;
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
    pub cancel: CancellationToken,
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
        let cancel = CancellationToken::new();
        let process = {
            to_owned![cancel];
            tokio::task::spawn_blocking(move || {
                match gstreamer_process::low_level::start_stream(video_device, output_path, cancel)
                {
                    Ok(_) => info!("process has finished"),
                    Err(message) => error!(?message, "bye bye"),
                }
            })
            .abort_on_drop()
        };
        Ok(Self { process, cancel })
    }

    #[tracing::instrument(ret, err, level = "INFO")]
    pub async fn wait_for_finish(self) -> Result<()> {
        {
            let cancel = self.cancel.clone();
            tokio::task::spawn(async move {
                while let Ok(()) = tokio::signal::ctrl_c().await {
                    tracing::warn!("CTRL+C received, shutting down");
                    cancel.cancel();
                }
            });
        }
        self.process
            .await
            .map_err(eyre::Report::from)
            .map_err(|v| v.wrap_err("waiting for gstreamer process to finish"))
    }
}
