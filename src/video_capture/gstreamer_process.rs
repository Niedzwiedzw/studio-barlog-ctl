use super::*;
use gst::{prelude::*, ElementFactory, Pipeline};
use gstreamer as gst;
use tracing::info;

#[instrument]
pub fn start_stream() -> Result<()> {
    gst::init()?;

    // Build the pipeline
    // let uri = "https://gstreamer.freedesktop.org/data/media/sintel_trailer-480p.webm";
    let pipeline = gst::parse_launch(&format!(
        "v4l2src device=/dev/video1 ! capsfilter caps=\"video/x-raw, width=1920, height=1080, format=YUY2, framerate=25/1\" ! autovideosink"
    ))?;

    // Start playing
    let res = pipeline.set_state(gst::State::Playing)?;
    let is_live = res == gst::StateChangeSuccess::NoPreroll;

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
                gst::MessageView::Buffering(buffering) => {
                    // If the stream is live, we do not care about buffering
                    if is_live {
                        return glib::ControlFlow::Continue;
                    }

                    let percent = buffering.percent();
                    print!("Buffering ({percent}%)\r");
                    match std::io::stdout().flush() {
                        Ok(_) => {}
                        Err(err) => eprintln!("Failed: {err}"),
                    };

                    // Wait until buffering is complete before start/resume playing
                    if percent < 100 {
                        let _ = pipeline.set_state(gst::State::Paused);
                    } else {
                        let _ = pipeline.set_state(gst::State::Playing);
                    }
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

    Ok(())
}
