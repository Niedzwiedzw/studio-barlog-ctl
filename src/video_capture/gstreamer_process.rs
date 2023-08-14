use super::*;
use gst::{prelude::*, ElementFactory, Pipeline};
use gstreamer as gst;
use tracing::info;

#[instrument]
pub fn start_stream() -> Result<()> {
    // Initialize GStreamer
    gst::init().wrap_err("initializing gstreamer")?;

    // Create a new pipeline
    let pipeline = Pipeline::new();

    // Create elements
    let src = ElementFactory::make("v4l2src").build()?;
    let convert = ElementFactory::make("videoconvert").build()?;
    let format = gst::Caps::builder("video/x-raw")
        .field("width", 1920i32)
        .field("height", 1080i32)
        .field("framerate", gst::Fraction::new(25, 1))
        .field("format", "I420")
        .build();
    //     .
    //     &[
    //         ("width", &1920i32),
    //         ("height", &1080i32),
    //         ("framerate", &gst::Fraction::new(30, 1)),
    //         ("format", &"I420"),
    //     ],
    // );
    let filter = ElementFactory::make("capsfilter")
        .property("caps", &format)
        .build()?;
    // filter.set_property("caps", &format)?;
    let encoder = ElementFactory::make("x264enc").build()?;
    encoder.set_property_from_str("bitrate", "8000");
    encoder.set_property_from_str("speed-preset", "ultrafast");
    encoder.set_property_from_str("tune", "zerolatency");

    let muxer = ElementFactory::make("matroskamux").build()?;
    let filesink = ElementFactory::make("filesink")
        .property("location", "output.mkv")
        .build()?;
    let rtpsink = ElementFactory::make("udpsink").build()?;
    rtpsink.set_property_from_str("host", "127.0.0.1");
    rtpsink.set_property_from_str("port", "5004");
    // Create the tee element

    let tee = ElementFactory::make("tee").build()?;

    // Add elements to the pipeline
    pipeline.add_many([
        &src, &convert, &filter, &encoder, &muxer, &tee, &filesink, &rtpsink,
    ])?;

    // Link the elements
    gst::Element::link_many([&src, &convert, &filter, &encoder, &muxer, &tee])?;

    // Link the first branch of the tee (save to file)
    let filesinkpad = tee
        .request_pad_simple("src_0")
        .ok_or_else(|| eyre!("no pad"))?;
    let queue = ElementFactory::make("queue").build()?;
    let muxerpad = queue
        .static_pad("sink")
        .ok_or_else(|| eyre!("no static pad (sink)"))?;
    queue.set_property_from_str("leaky", "downstream");
    pipeline.add(&queue)?;
    gst::Pad::link(&filesinkpad, &muxerpad)?;

    // Link the second branch of the tee (stream to VLC)
    let rtpsinkpad = tee
        .request_pad_simple("src_1")
        .ok_or_else(|| eyre!("no request pad (src1)"))?;
    let rtppay = ElementFactory::make("rtpvp8pay").build()?;
    pipeline.add(&rtppay)?;
    let rtpqueue = ElementFactory::make("queue").build()?;
    pipeline.add(&rtpqueue)?;
    let rtpmuxer = ElementFactory::make("rtpmux").build()?;
    pipeline.add(&rtpmuxer)?;
    gst::Pad::link(
        &rtpsinkpad,
        &rtppay
            .static_pad("sink")
            .ok_or_else(|| eyre!("no sink pad"))?,
    )?;
    gst::Element::link_many([&rtppay, &rtpqueue, &rtpmuxer, &rtpsink])?;

    // Set the pipeline to playing state
    pipeline.set_state(gst::State::Playing)?;

    // Run the main loop
    let main_loop = glib::MainLoop::new(None, false);
    main_loop.run();
    let bus = pipeline
        .bus()
        .ok_or_else(|| eyre!("pipeline must have a bus"))?;
    info!("starting to read messages");
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        info!(?msg, "new message");
    }

    // Clean up
    pipeline.set_state(gst::State::Null)?;

    Ok(())
}
