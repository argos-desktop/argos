use crate::session::{CaptureResult, Shared, Token};
use screencapturekit::{
    cm::SCFrameStatus,
    content_sharing_picker::{
        SCContentSharingPicker as SystemPicker, SCContentSharingPickerConfiguration,
        SCContentSharingPickerMode, SCPickerEvent, SCPickerResult, SCPickerSubscription,
    },
    prelude::*,
    stream::delegate_trait::StreamCallbacks,
};
use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};
use tauri::WebviewWindow;

pub struct Picker {
    events: Receiver<CaptureResult<Option<SCPickerResult>>>,
    _observer: SCPickerSubscription,
}

pub fn pick(_: &WebviewWindow) -> CaptureResult<Picker> {
    let mut config = SCContentSharingPickerConfiguration::new()
        .map_err(|_| "The macOS sharing picker is unavailable.")?;
    config.set_allowed_picker_modes(&[
        SCContentSharingPickerMode::SingleDisplay,
        SCContentSharingPickerMode::SingleWindow,
    ]);
    config.set_allows_changing_selected_content(false);
    SystemPicker::set_default_configuration(&config)
        .map_err(|_| "Could not configure the macOS picker.")?;
    let (tx, events) = mpsc::sync_channel(1);
    let observer = SystemPicker::add_observer(move |event| {
        let selection = match event {
            SCPickerEvent::Updated {
                result,
                stream: None,
            } => Ok(Some(result)),
            SCPickerEvent::Cancelled { stream: None } => Ok(None),
            SCPickerEvent::Failed(_) => Err("macOS could not open the picker."),
            _ => return,
        };
        let _ = tx.try_send(selection);
    })
    .map_err(|_| "Could not attach the macOS picker.")?;
    SystemPicker::present().map_err(|_| "Could not present the macOS picker.")?;
    Ok(Picker {
        events,
        _observer: observer,
    })
}

fn peak(samples: &[u8]) -> f32 {
    samples
        .chunks_exact(4)
        .map(|b| f32::from_ne_bytes(b.try_into().unwrap()).abs())
        .filter(|v| v.is_finite())
        .fold(0.0, f32::max)
        .min(1.0)
}

pub fn run(picker: Picker, state: &Shared, token: &Token) -> CaptureResult {
    let tick = Duration::from_millis(50);
    let selected = loop {
        if token.cancelled() {
            return Ok(());
        }
        match picker.events.recv_timeout(tick) {
            Ok(Ok(Some(result))) => break result,
            Ok(Ok(None)) => return Ok(()),
            Ok(Err(e)) => return Err(e),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("Lost connection to the macOS picker.");
            }
        }
    };
    if token.cancelled() {
        return Ok(());
    }
    let (width, height) = selected.pixel_size();
    if width == 0 || height == 0 {
        return Err("Selected source has no visible area.");
    }
    let config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_queue_depth(3)
        .with_minimum_frame_interval(&CMTime::new(1, 10))
        .with_captures_audio(true)
        .with_excludes_current_process_audio(true)
        .with_sample_rate(48000)
        .with_channel_count(1);
    let (ended, failure) = mpsc::sync_channel(1);
    let delegate = StreamCallbacks::new().on_error(move |_| {
        let _ = ended.try_send(());
    });
    let mut stream = SCStream::new_with_delegate(&selected.filter(), &config, delegate)
        .map_err(|_| "Could not prepare macOS capture.")?;
    let video_session = state.clone();
    let video_token = token.clone();
    stream
        .add_output_handler(
            move |sample: CMSampleBuffer, _| {
                if sample.frame_status() == Some(SCFrameStatus::Complete)
                    && let Some(buffer) = sample.pixel_buffer()
                {
                    video_session.lock().unwrap().frame(
                        &video_token,
                        buffer.width() as i32,
                        buffer.height() as i32,
                    );
                }
            },
            SCStreamOutputType::Screen,
        )
        .map_err(|_| "Could not attach screen capture.")?;
    let audio_session = state.clone();
    let audio_token = token.clone();
    stream
        .add_output_handler(
            move |sample: CMSampleBuffer, _| {
                if let Ok(buffers) = sample.audio_buffer_list() {
                    let level = (&buffers)
                        .into_iter()
                        .map(|b| peak(b.data()))
                        .fold(0.0, f32::max);
                    audio_session.lock().unwrap().audio(
                        &audio_token,
                        sample.num_samples().max(0) as u32,
                        level,
                    );
                }
            },
            SCStreamOutputType::Audio,
        )
        .map_err(|_| "Could not attach playback capture.")?;
    if token.cancelled() {
        return Ok(());
    }
    stream
        .start_capture()
        .map_err(|_| "Capture could not start. Check macOS sharing permissions and try again.")?;
    state.lock().unwrap().running(token);
    while !token.cancelled() {
        if !matches!(failure.recv_timeout(tick), Err(RecvTimeoutError::Timeout)) {
            return Err("macOS sharing ended. Choose a source to restart.");
        }
    }
    stream
        .stop_capture()
        .map_err(|_| "Could not stop macOS capture cleanly.")
}

#[cfg(test)]
mod tests {
    use super::peak;
    #[test]
    fn pcm_meter_handles_silence_polarity_and_clipping() {
        assert_eq!(peak(&[0; 16]), 0.0);
        let bytes: Vec<_> = [0.25f32, -0.75, f32::NAN]
            .into_iter()
            .flat_map(f32::to_ne_bytes)
            .collect();
        assert_eq!(peak(&bytes), 0.75);
        assert_eq!(peak(&1.2f32.to_ne_bytes()), 1.0);
    }
}
