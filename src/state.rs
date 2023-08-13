use super::*;
use crate::space_available_watcher::SpaceAvailableWatcher;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tui::layout::Rect;

pub struct StudioState {
    pub wake_up: Option<UnboundedReceiverStream<ProcessEvent>>,
    reaper: ReaperInstance,
    qpwgraph: QpwgraphInstance,
    ffmpeg: FfmpegInstance,
    space_available: SpaceAvailableWatcher,
}

mod dynamic_template {
    use std::iter::once;

    use reaper_save_rs::{
        low_level::{Attribute, Object, ReaperString},
        prelude::{ObjectWrapper, ReaperProject, SerializeAndDeserialize, Track},
    };

    use super::*;
    fn template_video_track() -> Result<Track> {
        let template_video_track_code = r#"
<TRACK {E2DDC8BD-1B29-165D-D141-7267E8F39ECD}
  NAME "VIDEO 1"
  PEAKCOL 16576
  BEAT -1
  AUTOMODE 0
  VOLPAN 1 0 -1 -1 1
  MUTESOLO 0 0 0
  IPHASE 0
  PLAYOFFS 0 1
  ISBUS 0 0
  BUSCOMP 0 0 0 0 0
  SHOWINMIX 1 0.558065 0.5 1 0.5 0 0 0
  SEL 0
  REC 0 0 0 0 0 0 0 0
  VU 2
  TRACKHEIGHT 0 0 0 0 0 0
  INQ 0 0 0 0.5 100 0 0 100
  NCHAN 2
  FX 1
  TRACKID {E2DDC8BD-1B29-165D-D141-7267E8F39ECD}
  PERF 0
  MIDIOUT -1
  MAINSEND 1 0
  <ITEM
    POSITION 0
    SNAPOFFS 0
    LENGTH 188.04
    LOOP 1
    ALLTAKES 0
    FADEIN 1 0.01 0 1 0 0 0
    FADEOUT 1 0.01 0 1 0 0 0
    MUTE 0 0
    SEL 1
    IGUID {2F6AD700-840B-EFB6-D384-7F8316E1C1E7}
    IID 21
    NAME barbarah-anne---2023-07-31--20-51-57.mov
    VOLPAN 1 0 1 -1
    SOFFS 0
    PLAYRATE 1 0 0 -1 0 0.0025
    CHANMODE 0
    GUID {A365E92F-3BF8-24E8-1FF4-8FDF30208BCB}
    <SOURCE VIDEO
      FILE "video-recordings/barbarah-anne---2023-07-31--20-51-57.mov"
    >
  >
>
        "#
        .trim();
        Object::deserialize(template_video_track_code, 0)
            .wrap_err("deserializing template track")
            .and_then(|(_, o)| Track::from_object(o).wrap_err("validating input"))
    }

    fn double_quote(val: &str) -> Attribute {
        Attribute::String(ReaperString::DoubleQuote(val.to_owned()))
    }

    pub fn append_video_to(
        mut reaper_project: ReaperProject,
        video_path: PathBuf,
    ) -> Result<ReaperProject> {
        template_video_track()
            .and_then(|mut track| -> Result<_> {
                // asd
                let file_path = double_quote(video_path.clone().display().to_string().as_str());
                *track.as_mut().single_attribute_mut("NAME")? = file_path.clone();
                let item = track
                    .as_mut()
                    .child_object_mut("ITEM")
                    .ok_or_else(|| eyre!("no ITEM in template"))?;
                *item.single_attribute_mut("NAME")? = file_path.clone();
                let source = item
                    .child_object_mut("SOURCE")
                    .ok_or_else(|| eyre!("no SOURCE in ITEM"))?;
                *source.single_attribute_mut("FILE")? = file_path;
                Ok(track)
            })
            .wrap_err("creating video track")
            .and_then(|video_track| {
                reaper_project
                    .modify_tracks(|tracks| {
                        tracks
                            .into_iter()
                            .chain(once(video_track.clone()))
                            .collect()
                    })
                    .wrap_err("modifying tracks")
            })
            .map(move |_| reaper_project)
    }

    pub fn with_video_track(template_path: PathBuf, video_path: PathBuf) -> Result<PathBuf> {
        std::fs::read_to_string(template_path)
            .wrap_err("reading original")
            .and_then(|original| {
                ReaperProject::parse_from_str(&original).wrap_err("parsing original")
            })
            .and_then(|parsed| append_video_to(parsed, video_path))
            .and_then(|modified| modified.serialize_to_string().wrap_err("serializing"))
            .and_then(|serialized| {
                directory_shenanigans::temp_home_path("generated-template.rpp").and_then(|path| {
                    std::fs::write(&path, serialized)
                        .wrap_err_with(|| format!("writing to {}", path.display()))
                        .map(|_| path)
                })
            })
    }
}

impl StudioState {
    pub async fn new(
        sessions_directory: SessionsDirectory,
        project_name: ProjectName,
        template: PathBuf,
        reaper_web_base_url: reqwest::Url,
        loopback_device: LoopbackDevice,
    ) -> Result<Self> {
        let (notify, wake_up) = tokio::sync::mpsc::unbounded_channel();
        let qpwgraph = crate::qpwgraph::QpwgraphInstance::new(notify.clone())
            .await
            .wrap_err("Spawning qpwgraph")?;

        let ffmpeg = FfmpegInstance::new(
            sessions_directory.clone(),
            loopback_device,
            project_name.clone(),
            notify.clone(),
        )
        .map(|v| v.wrap_err("spawning ffmpeg"))
        .await?;
        let space_available =
            SpaceAvailableWatcher::new(sessions_directory.as_ref().as_ref().to_owned());

        let template_with_video =
            dynamic_template::with_video_track(template, ffmpeg.video_file_path.clone())?;
        let reaper = crate::reaper::ReaperInstance::new(
            sessions_directory,
            project_name.clone(),
            template_with_video,
            notify.clone(),
            reaper_web_base_url,
        )
        .map(|v| v.wrap_err("starting reaper"))
        .await?;
        Ok(Self {
            space_available,
            wake_up: Some(UnboundedReceiverStream::new(wake_up)),
            reaper,
            qpwgraph,
            ffmpeg,
        })
    }
}

impl crate::rendering::RenderToTerm for StudioState {
    fn render_to_term<B: Backend>(
        &mut self,
        frame: &mut Frame<B>,
        rect: tui::layout::Rect,
    ) -> Result<()> {
        let Self {
            wake_up: _,
            reaper,
            qpwgraph,
            ffmpeg,
            space_available,
        } = self;
        let [header, body]: [Rect; 2] = layout!(Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(10), Constraint::Percentage(90)].as_ref())
            .split(rect));
        let [qpwgraph_col, reaper_col, ffmpeg_frame]: [Rect; 3] = layout!(Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(body));

        space_available.render_to_term(frame, header)?;
        qpwgraph.render_to_term(frame, qpwgraph_col)?;
        reaper.render_to_term(frame, reaper_col)?;
        ffmpeg.render_to_term(frame, ffmpeg_frame)?;

        Ok(())
    }
}
