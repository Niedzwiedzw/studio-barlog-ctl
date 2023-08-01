use std::{future::ready, sync::Arc};


use futures::TryFutureExt;
use itertools::Itertools;
use reqwest::Url;


use crate::directory_shenanigans::project_directory;

use super::*;

#[derive(Debug, Clone)]
pub struct ReaperInstance {
    process: Arc<RwLock<ProcessWatcher>>,
    web_client: Arc<ReaperWebClient>,
}

/// uses barely-documented web api, it's pretty simple though
#[derive(Debug, Clone)]
pub struct ReaperWebClient {
    client: reqwest::Client,
    base_addr: Url,
}

impl ReaperWebClient {
    pub async fn new(base_addr: Url) -> Result<Arc<Self>> {
        ready(
            reqwest::ClientBuilder::new()
                .build()
                .wrap_err("building http client")
                .map(|client| Self { client, base_addr })
                .map(Arc::new),
        )
        .and_then(|client| client.wait_alive())
        .await
    }
    #[instrument(skip(self), level = "info", ret, err)]
    pub async fn run_command(self: Arc<Self>, commands: &[&str]) -> Result<()> {
        ready(
            self.base_addr
                .join(&format!(
                    "_/{commands};",
                    commands = commands.iter().map(|v| v.to_string()).join(";")
                ))
                .wrap_err("invalid url"),
        )
        .and_then(|url| {
            self.client
                .get(url)
                .send()
                .map(|r| r.wrap_err("sending command request"))
                .and_then(|res| ready(res.error_for_status().wrap_err("invalid status")))
                .map_ok(|_| ())
        })
        .await
    }
    async fn is_alive(self: Arc<Self>) -> Result<Arc<Self>> {
        self.clone()
            .run_command(&["TRANSPORT"])
            .map_ok(|_| self)
            .await
    }

    async fn wait_alive(self: Arc<Self>) -> Result<Arc<Self>> {
        const SECONDS: u64 = 30;
        for _ in 0..(SECONDS * 10) {
            match self.clone().is_alive().await {
                Ok(alive) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    return Ok(alive);
                }
                Err(_) => tokio::time::sleep(tokio::time::Duration::from_millis(100)).await,
            }
        }
        bail!("reaper is probably not gonna start")
    }
    pub async fn start_reaper_recording(self: Arc<Self>) -> Result<()> {
        self.run_command(&["1013"]).await
    }
}

impl ReaperInstance {
    #[instrument(ret, err)]
    pub async fn new(
        project_name: ProjectName,
        template: PathBuf,
        notify: ProcessEventBus,
        web_client_base_address: reqwest::Url,
    ) -> Result<Self> {
        let process_path = "reaper";

        ready(project_directory(&project_name))
            .and_then(move |project_directory| {
                ready(
                    bounded_command(process_path)
                        .arg("-new")
                        .arg("-saveas")
                        .arg(
                            project_directory
                                .as_ref()
                                .join(format!("{project_name}.rpp")),
                        )
                        .arg("-template")
                        .arg(&template)
                        .arg("-nosplash")
                        .env("PIPEWIRE_LATENCY", "128/48000")
                        .spawn()
                        .wrap_err("spawning process instance")
                        .map(|child| {
                            ProcessWatcher::new(
                                process_path.to_owned(),
                                child.gracefully_shutdown_on_drop(),
                                notify,
                            )
                        })
                        .map(RwLock::new)
                        .map(Arc::new),
                )
            })
            .and_then(|child| {
                ReaperWebClient::new(web_client_base_address).and_then(|web_client| {
                    web_client
                        .clone()
                        .start_reaper_recording()
                        .map_ok(|_| child)
                        .map(|child| {
                            child.map(|process| Self {
                                process,
                                web_client,
                            })
                        })
                })
            })
            .await
    }
}

impl RenderToTerm for ReaperInstance {
    fn render_to_term<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        self.process.write().render_to_term(f, rect)?;

        Ok(())
    }
}
