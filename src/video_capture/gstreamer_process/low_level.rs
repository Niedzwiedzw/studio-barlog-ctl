use super::*;
use gst::{prelude::*, ElementFactory, Pipeline};
use gstreamer as gst;
use tracing::{info, warn};
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FORMAT: &str = "YUY2";
const FRAMERATE: &str = "25/1";

// #[instrument(ret, err, level = "INFO")]
// fn construct_pipeline(video_device: VideoDevice, output_file: PathBuf) -> Result<gst::Pipeline> {
//     let pipeline = gst::Pipeline::with_name("preview-and-record-pipeline");
//     let video_source = gst::ElementFactory::make("v4l2src")
//         .property("device", video_device.to_string())
//         .build()
//         .wrap_err("building video source element")?;

//     // let capsfilter = gst::ElementFactory::make("capsfilter")
//     //     .property(
//     //         "caps",
//     //         &gst::Caps::builder("video/x-raw")
//     //             .field("width", WIDTH)
//     //             .field("height", HEIGHT)
//     //             .field("format", FORMAT)
//     //             .field("framerate", FRAMERATE)
//     //             .build(),
//     //     )
//     //     .build()
//     //     .wrap_err("building capsfilter element")?;
//     let tee = gst::ElementFactory::make("tee")
//         .build()
//         .wrap_err("building tee element")?;
//     let queue1 = gst::ElementFactory::make("queue")
//         .build()
//         .wrap_err("building queue1 element")?;
//     let queue2 = gst::ElementFactory::make("queue")
//         .build()
//         .wrap_err("building queue2 element")?;
//     let video_convert = gst::ElementFactory::make("videoconvert")
//         .build()
//         .wrap_err("building video_convert element")?;
//     let x264enc = gst::ElementFactory::make("x264enc")
//         .property("bitrate", 8000u32)
//         .property_from_str("speed-preset", "ultrafast")
//         .property_from_str("tune", "zerolatency")
//         .build()
//         .wrap_err("building x264enc element")?;
//     let matroskamux = gst::ElementFactory::make("matroskamux")
//         .build()
//         .wrap_err("building matroskamux element")?;
//     let filesink = gst::ElementFactory::make("filesink")
//         .property("location", output_file.display().to_string())
//         .build()
//         .wrap_err("building filesink element")?;
//     let autovideosink = gst::ElementFactory::make("autovideosink")
//         .build()
//         .wrap_err("building autovideosink")?;

//     // Build pipeline
//     pipeline
//         .add_many([
//             &video_source,
//             &capsfilter,
//             &tee,
//             &queue1,
//             &queue2,
//             &video_convert,
//             &x264enc,
//             &matroskamux,
//             &filesink,
//             &autovideosink,
//         ])
//         .wrap_err("adding elements to pipeline")?;

//     video_source
//         .link(&capsfilter)
//         .wrap_err("connecting video source to capsfilter")?;
//     // Link elements
//     // gst::Element::link_many([&video_source, &capsfilter, &tee])
//     //     .wrap_err("linking elements to tee")?;

//     gst::Element::link_many([&queue1, &autovideosink]).wrap_err("linking queue1 elements")?;
//     gst::Element::link_many([&queue2, &video_convert, &x264enc, &matroskamux, &filesink])
//         .wrap_err("linking queue elements")?;
//     tee.link(&queue1).wrap_err("linking queue1 to tee")?;
//     tee.link(&queue2).wrap_err("linking queue2 to tee")?;
//     Ok(pipeline)
// }

#[instrument(ret, err, level = "INFO")]
pub fn start_stream(
    video_device: VideoDevice,
    output_file: PathBuf,
    cancel: CancellationToken,
) -> Result<()> {
    gst::init()?;
    // gst-launch-1.0 -e  v4l2src device=/dev/video1 !  videoconvert !  video/x-raw,width=1920,height=1080,framerate=25/1,format=I420 !  x264enc bitrate=8000 speed-preset=ultrafast tune=zerolatency !  video/x-h264 !  matroskamux !  filesink location=output.mkv

    // Build the pipeline
    // let uri = "https://gstreamer.freedesktop.org/data/media/sintel_trailer-480p.webm";
    const VIDEO_SOURCE: &str = "video-source";
    let pipeline_str = format!(
        r#"
    v4l2src device={video_device} name="{VIDEO_SOURCE}"
        ! capsfilter caps="video/x-raw, width={WIDTH}, height={HEIGHT}, format={FORMAT}, framerate={FRAMERATE}"
        ! tee name=t
            t. ! queue ! autovideosink
            t. ! queue
                ! videoconvert
                ! x264enc bitrate=8000 speed-preset=ultrafast tune=zerolatency
                ! video/x-h264
                ! matroskamux
                ! filesink location={output_file}
            "#,
        output_file = output_file.display()
    );
    info!(%pipeline_str);
    // let pipeline =
    //     construct_pipeline(video_device, output_file).wrap_err("constructing pipeline")?;
    let pipeline = gst::parse_launch(&pipeline_str)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| eyre!("invalid pipeline object"))
        .wrap_err("invalid pipeline")?;
    let video_source = pipeline
        .by_name(VIDEO_SOURCE)
        .ok_or_else(|| eyre!("element {VIDEO_SOURCE} not properly setup"))?;

    // Start playing
    let _res = pipeline.set_state(gst::State::Playing)?;
    let cancel_watcher = std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if cancel.is_cancelled() {
            video_source.send_event(gst::event::Eos::new());
            break;
        }
    });

    let main_loop = glib::MainLoop::new(None, false);
    let main_loop_clone = main_loop.clone();
    let pipeline_weak = pipeline.downgrade();
    let bus = pipeline.bus().expect("Pipeline has no bus");
    let _bus_watch = bus
        .add_watch(move |_, msg| {
            let pipeline = match pipeline_weak.upgrade() {
                Some(pipeline) => pipeline,
                None => return glib::ControlFlow::Continue,
            };
            let main_loop = &main_loop_clone;
            match msg.view() {
                gst::MessageView::Error(err) => {
                    println!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    let _ = pipeline.set_state(gst::State::Ready);
                    main_loop.quit();
                }
                gst::MessageView::Eos(..) => {
                    // end-of-stream
                    let _ = pipeline.set_state(gst::State::Ready);
                    main_loop.quit();
                }
                gst::MessageView::ClockLost(_) => {
                    // Get a new clock
                    let _ = pipeline.set_state(gst::State::Paused);
                    let _ = pipeline.set_state(gst::State::Playing);
                }
                _ => (),
            }
            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    main_loop.run();

    pipeline.set_state(gst::State::Null)?;
    if let Err(join_error) = cancel_watcher
        .join()
        .map_err(|e| eyre!("{e:#?}"))
        .wrap_err("shutting down watcher thread")
    {
        warn!(?join_error, "failed to shut down the watcher thread");
    }

    Ok(())
}
