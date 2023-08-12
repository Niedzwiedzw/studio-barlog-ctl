use self::rea_request::{ActionId, ReaRequest, ReaResponse};

use super::*;
/// uses barely-documented web api, it's pretty simple though
#[derive(Debug, Clone)]
pub struct ReaperWebClient {
    client: reqwest::Client,
    base_addr: Url,
}

pub mod rea_request {
    use std::str::FromStr;

    use crate::reaper::common_types::ReaperBool;

    use super::*;
    pub trait ReaResponse: Sized {
        fn from_response(response: &str) -> Result<Self>;
    }

    pub trait ReaRequest {
        type Response: ReaResponse;
        fn as_uri(&self) -> String {
            std::any::type_name::<Self>()
                .split("::")
                .last()
                .unwrap_or_default()
                .to_uppercase()
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, strum::FromRepr)]
    pub enum Playstate {
        Stopped = 0,
        Playing = 1,
        Paused = 3,
        Recording = 5,
        RecordPaused = 6,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, strum::FromRepr)]
    pub enum ActionId {
        TransportRecord = 1013,
        TransportStop = 1016,
        SaveProject = 40026,
        /// Item navigation: Move cursor to end of items
        ItemNavigationMoveCursorToEndOfItems = 41174,
    }

    macro_rules! from_repr {
        ($ty:ty) => {
            impl FromStr for $ty {
                type Err = eyre::Report;

                fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
                    s.parse()
                        .wrap_err_with(|| eyre!("invalid number: '{s}'"))
                        .and_then(|num| {
                            Self::from_repr(num).ok_or_else(|| eyre!("invalid repr {num}"))
                        })
                        .wrap_err_with(|| {
                            format!("parsing value of type {}", std::any::type_name::<Self>())
                        })
                }
            }
        };
    }
    from_repr!(Playstate);
    from_repr!(ReaperBool);
    from_repr!(ActionId);

    macro_rules! rea_response {
    (
        $(#[$attr:meta])*
        $vis:vis struct $name:ident {
            $($field_vis:vis $field:ident : $ty:ty),* $(,)?
        }
    ) => {
        $(#[$attr])*
        $vis struct $name {
            $(
                $field_vis $field: $ty,
            )*
        }

        impl ReaResponse for $name {
            #[instrument(err, ret, level = "debug")]
            fn from_response(response: &str) -> Result<Self> {
                let mut fields = response.split("\t").skip(1);
                Ok(Self {
                    $(
                        $field: fields
                            .next()
                            .ok_or_else(|| eyre!("field missing"))
                            .and_then(|field| field.parse::<$ty>().wrap_err_with(|| format!("unexpected value {}", field)))
                            .wrap_err_with(|| format!("parsing {} ({})", stringify!($field), std::any::type_name::<$ty>()))
                            .wrap_err_with(|| format!("parsing response {}", std::any::type_name::<$name>()))?,
                    )*
                })
            }
        }
    };
}

    impl ReaResponse for () {
        fn from_response(_response: &str) -> Result<Self> {
            Ok(())
        }
    }

    /// * TRANSPORT
    /// Returns a line including (note that the spaces are here for readability, there
    /// is actually only tabs between fields):
    ///   TRANSPORT \t playstate \t position_seconds \t isRepeatOn \t position_string \t position_string_beats
    ///
    /// playstate is 0 for stopped, 1 for playing, 2 for paused, 5 for recording, and 6 for record paused.
    pub struct Transport;
    impl ReaRequest for Transport {
        type Response = TransportResponse;
    }

    impl ReaRequest for ActionId {
        type Response = ();
        fn as_uri(&self) -> String {
            (*self as isize).to_string()
        }
    }
    rea_response! {
        #[derive(Debug, Clone)]
        pub struct TransportResponse {
            pub playstate: Playstate,
            pub position_seconds: f64,
            pub is_repeat_on: ReaperBool,
            pub position_string: String,
            pub position_string_beats: String,
        }
    }
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
    pub async fn run_single<R: ReaRequest>(self: Arc<Self>, request: R) -> Result<R::Response> {
        ready(
            self.base_addr
                .join(&format!("_/{};", request.as_uri()))
                .wrap_err("invalid url"),
        )
        .and_then(|url| {
            self.client
                .get(url.clone())
                .send()
                .map(|r| r.wrap_err("sending command request"))
                .and_then(|res| ready(res.error_for_status().wrap_err("invalid status")))
                .and_then(|r| r.text().map(|res| res.wrap_err("bad text response")))
                .and_then(|response| {
                    ready(
                        ReaResponse::from_response(&response)
                            .wrap_err_with(|| format!("bad response: '{response}'")),
                    )
                })
                .map(move |v| v.wrap_err_with(|| format!("performing request: {url}")))
        })
        .await
        .wrap_err_with(|| format!("performing reaper request: {}", std::any::type_name::<R>()))
    }

    async fn is_alive(self: Arc<Self>) -> Result<Arc<Self>> {
        self.clone()
            .run_single(rea_request::Transport)
            .map_ok(|_| self)
            .await
    }

    async fn wait_alive(self: Arc<Self>) -> Result<Arc<Self>> {
        const SECONDS: u64 = 5;
        let mut last_err: Option<eyre::Report> = None;
        for _ in 0..(SECONDS * 10) {
            match self.clone().is_alive().await {
                Ok(alive) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    return Ok(alive);
                }
                Err(report) => {
                    let _ = last_err.insert(report);
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
        bail!("healthcheck failed: {last_err:?}")
    }
    pub async fn start_reaper_recording(self: Arc<Self>) -> Result<()> {
        self.run_single(ActionId::TransportRecord).await
    }
}
