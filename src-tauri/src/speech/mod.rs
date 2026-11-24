pub mod model;

use std::{
    collections::VecDeque,
    f32::consts::PI,
    path::Path,
    time::{Duration, Instant},
};
use transcribe_cpp::{
    Model, MoonshineStreamingOptions, RunOptions, Session, Stream, StreamExtension, StreamOptions,
};

const FRAME: usize = 256;
const FRAME_TIME: Duration = Duration::from_millis(16);
const END_OF_TURN_FRAMES: usize = 28;
const MIN_VOICED_FRAMES: usize = 10;
const PREROLL: usize = 20 * FRAME;
const BATCH: usize = 20 * FRAME;
// The model drops audio past its input limit, and the final decode grows with the turn.
const MAX_TURN_FRAMES: usize = 1875;
const TAPS: usize = 96;

pub struct Turn {
    pub text: String,
    pub speech_ended: Instant,
}

type Vad = Box<dyn FnMut(&[f32]) -> bool + Send>;

trait Transcribe {
    fn begin(&mut self);
    fn feed(&mut self, pcm_16k: &[f32]);
    fn finish(&mut self) -> String;
}

struct Resampler {
    taps: [f32; TAPS],
    input: Vec<f32>,
}

impl Resampler {
    fn new() -> Self {
        let mut taps: [f32; TAPS] = std::array::from_fn(|i| {
            let t = i as f32 - (TAPS - 1) as f32 / 2.0;
            let x = 2.0 * PI * t * 7100.0 / 48000.0;
            let window = 0.54 - 0.46 * (2.0 * PI * i as f32 / (TAPS - 1) as f32).cos();
            window * x.sin() / x
        });
        let gain: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|tap| *tap /= gain);
        Self {
            taps,
            // Primed so n input samples always yield n / 3 outputs.
            input: vec![0.0; TAPS - 3],
        }
    }

    fn push(&mut self, pcm_48k: &[f32], out: &mut Vec<f32>) {
        self.input.extend_from_slice(pcm_48k);
        let mut at = 0;
        while at + TAPS <= self.input.len() {
            let window = &self.input[at..at + TAPS];
            out.push(window.iter().zip(&self.taps).map(|(s, t)| s * t).sum());
            at += 3;
        }
        self.input.drain(..at);
    }
}

pub struct Speech {
    resampler: Resampler,
    audio: Vec<f32>,
    vad: Vad,
    asr: Box<dyn Transcribe + Send>,
    // Pre-roll while idle, then the unconfirmed start of a possible turn.
    pending: Vec<f32>,
    voiced: usize,
    silent: usize,
    length: usize,
    ready: VecDeque<Turn>,
}

impl Speech {
    pub fn open(model: &Path) -> Result<Speech, &'static str> {
        const FAILED: &str = "Could not load the speech model.";
        transcribe_cpp::disable_logging();
        let model = Model::load(model).map_err(|_| FAILED)?;
        let capabilities = model.capabilities().map_err(|_| FAILED)?;
        let limit = Duration::from_millis(capabilities.max_audio_ms as u64);
        if !capabilities.supports_streaming
            || (!limit.is_zero()
                && limit < FRAME_TIME * (MAX_TURN_FRAMES + 2 * BATCH / FRAME) as u32)
        {
            return Err(FAILED);
        }
        let session = Box::into_raw(Box::new(model.session().map_err(|_| FAILED)?));
        let mut detector = earshot::Detector::default_boxed();
        Ok(Self::new(
            Box::new(move |frame| detector.predict_f32(frame) > 0.5),
            Box::new(Engine {
                stream: None,
                session,
            }),
        ))
    }

    fn new(vad: Vad, asr: Box<dyn Transcribe + Send>) -> Speech {
        Speech {
            resampler: Resampler::new(),
            audio: Vec::new(),
            vad,
            asr,
            pending: Vec::new(),
            voiced: 0,
            silent: 0,
            length: 0,
            ready: VecDeque::new(),
        }
    }

    pub fn feed(&mut self, pcm_48k_mono: &[f32]) -> Option<Turn> {
        self.resampler.push(pcm_48k_mono, &mut self.audio);
        let audio = std::mem::take(&mut self.audio);
        let mut frames = audio.chunks_exact(FRAME);
        for frame in &mut frames {
            self.frame(frame);
        }
        self.audio.extend_from_slice(frames.remainder());
        self.ready.pop_front()
    }

    /// For capture backends that deliver nothing while the far side is quiet.
    pub fn silence(&mut self, elapsed: Duration) -> Option<Turn> {
        // A second is enough to end any open turn; more would only allocate.
        let samples = elapsed.min(Duration::from_secs(1)).as_secs_f32() * 48000.0;
        self.feed(&vec![0.0; samples as usize])
    }

    fn frame(&mut self, frame: &[f32]) {
        self.pending.extend_from_slice(frame);
        if (self.vad)(frame) {
            self.voiced += 1;
            self.silent = 0;
            if self.voiced == MIN_VOICED_FRAMES {
                self.asr.begin();
            }
        } else if self.voiced > 0 {
            self.silent += 1;
        }
        if self.voiced > 0 {
            self.length += 1;
        }
        let confirmed = self.voiced >= MIN_VOICED_FRAMES;
        let ended = self.silent == END_OF_TURN_FRAMES || self.length == MAX_TURN_FRAMES;
        // The encoder re-reads a fixed window of context on every call, so small feeds are costly.
        if confirmed && (self.pending.len() >= BATCH || ended) {
            self.asr.feed(&self.pending);
            self.pending.clear();
        }
        if ended {
            if confirmed {
                let speech_ended = Instant::now() - FRAME_TIME * self.silent as u32;
                let text = self.asr.finish();
                if !text.trim().is_empty() {
                    self.ready.push_back(Turn {
                        text: text.trim().to_owned(),
                        speech_ended,
                    });
                }
            }
            self.voiced = 0;
            self.silent = 0;
            self.length = 0;
        }
        if self.voiced == 0 {
            let excess = self.pending.len().saturating_sub(PREROLL);
            self.pending.drain(..excess);
        }
    }
}

struct Engine {
    stream: Option<Stream<'static>>,
    session: *mut Session,
}

// SAFETY: `Session` is `Send`, and the pointer is uniquely owned by this value.
unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stream = None;
        // SAFETY: allocated by `Box::into_raw` in `Speech::open`; the only borrower is gone.
        drop(unsafe { Box::from_raw(self.session) });
    }
}

impl Transcribe for Engine {
    fn begin(&mut self) {
        self.stream = None;
        // SAFETY: the session outlives the stream (see `Drop`) and is only reached through it.
        let session = unsafe { &mut *self.session };
        // Every partial decode re-reads the whole turn, and nothing here shows partials, so the
        // only decode is the one `finalize` runs.
        let options = StreamOptions {
            family: Some(StreamExtension::MoonshineStreaming(
                MoonshineStreamingOptions {
                    min_decode_interval_ms: Some(3_600_000),
                },
            )),
            ..Default::default()
        };
        self.stream = session.stream(&RunOptions::default(), &options).ok();
    }

    fn feed(&mut self, pcm_16k: &[f32]) {
        if self
            .stream
            .as_mut()
            .is_some_and(|s| s.feed(pcm_16k).is_err())
        {
            self.stream = None;
        }
    }

    fn finish(&mut self) -> String {
        let Some(mut stream) = self.stream.take() else {
            return String::new();
        };
        match stream.finalize() {
            Ok(_) => stream.text().full,
            Err(_) => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(usize);

    impl Transcribe for Fake {
        fn begin(&mut self) {
            self.0 = 0;
        }
        fn feed(&mut self, pcm_16k: &[f32]) {
            self.0 += pcm_16k.len();
        }
        fn finish(&mut self) -> String {
            format!("turn {}", self.0)
        }
    }

    struct Blank;

    impl Transcribe for Blank {
        fn begin(&mut self) {}
        fn feed(&mut self, _: &[f32]) {}
        fn finish(&mut self) -> String {
            " \n".into()
        }
    }

    fn speech(asr: impl Transcribe + Send + 'static) -> Speech {
        Speech::new(
            Box::new(|frame| frame.iter().map(|s| s.abs()).sum::<f32>() > 0.1 * FRAME as f32),
            Box::new(asr),
        )
    }

    fn signal(parts: &[(bool, u32)]) -> Vec<f32> {
        let mut pcm = Vec::new();
        for &(tone, ms) in parts {
            pcm.extend((0..ms * 48).map(|i| {
                if tone {
                    0.5 * (2.0 * PI * 440.0 * i as f32 / 48000.0).sin()
                } else {
                    0.0
                }
            }));
        }
        pcm
    }

    fn turns(pcm: &[f32], chunk: usize) -> Vec<String> {
        let mut speech = speech(Fake(0));
        let mut turns: Vec<String> = pcm
            .chunks(chunk)
            .filter_map(|c| speech.feed(c))
            .map(|turn| turn.text)
            .collect();
        while let Some(turn) = speech.feed(&[]) {
            turns.push(turn.text);
        }
        turns
    }

    #[test]
    fn resampler_outputs_a_third_of_its_input() {
        let mut resampler = Resampler::new();
        let mut out = Vec::new();
        for chunk in signal(&[(true, 1000)]).chunks(441) {
            resampler.push(chunk, &mut out);
        }
        assert_eq!(out.len(), 16000);
        let peak = out[1000..].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01, "{peak}");
    }

    #[test]
    fn resampler_rejects_content_above_the_new_nyquist() {
        let mut resampler = Resampler::new();
        let mut out = Vec::new();
        let pcm: Vec<f32> = (0..48000)
            .map(|i| (2.0 * PI * 12000.0 * i as f32 / 48000.0).sin())
            .collect();
        resampler.push(&pcm, &mut out);
        let peak = out[1000..].iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak < 0.01, "{peak}");
    }

    #[test]
    fn a_burst_then_silence_is_one_turn() {
        let mut speech = speech(Fake(0));
        assert!(speech.feed(&signal(&[(true, 1000)])).is_none());
        let turn = speech.feed(&signal(&[(false, 1000)])).unwrap();
        assert!(turn.text.starts_with("turn "));
        assert!(turn.speech_ended < Instant::now());
        assert!(speech.feed(&signal(&[(false, 1000)])).is_none());
    }

    #[test]
    fn a_turn_includes_its_preroll_and_trailing_silence() {
        let samples: usize = turns(&signal(&[(false, 1000), (true, 1000), (false, 1000)]), 480)[0]
            .trim_start_matches("turn ")
            .parse()
            .unwrap();
        let expected = PREROLL + 16000 + END_OF_TURN_FRAMES * FRAME;
        assert!(samples.abs_diff(expected) <= 2 * FRAME, "{samples}");
    }

    #[test]
    fn a_short_blip_is_not_a_turn() {
        assert!(turns(&signal(&[(true, 80), (false, 2000)]), 480).is_empty());
    }

    #[test]
    fn long_silence_is_not_a_turn() {
        assert!(turns(&signal(&[(false, 5000)]), 480).is_empty());
    }

    #[test]
    fn two_bursts_are_two_turns() {
        let pcm = signal(&[(true, 800), (false, 1000), (true, 600), (false, 1000)]);
        assert_eq!(turns(&pcm, 480).len(), 2);
    }

    #[test]
    fn a_pause_shorter_than_the_end_of_turn_does_not_split() {
        let pcm = signal(&[(true, 800), (false, 250), (true, 600), (false, 1000)]);
        assert_eq!(turns(&pcm, 480).len(), 1);
    }

    #[test]
    fn unbroken_speech_is_split_at_the_turn_limit() {
        assert_eq!(
            turns(&signal(&[(true, 62000), (false, 1000)]), 4800).len(),
            3
        );
    }

    #[test]
    fn chunk_boundaries_do_not_change_the_result() {
        let pcm = signal(&[
            (false, 300),
            (true, 800),
            (false, 1000),
            (true, 80),
            (false, 700),
            (true, 600),
            (false, 1000),
        ]);
        let whole = turns(&pcm, pcm.len());
        assert_eq!(whole.len(), 2);
        for chunk in [7, 441, 480, 4800, 50000] {
            assert_eq!(turns(&pcm, chunk), whole, "chunk {chunk}");
        }
    }

    #[test]
    fn blank_transcripts_are_suppressed() {
        let pcm = signal(&[(true, 1000), (false, 1000)]);
        assert!(speech(Blank).feed(&pcm).is_none());
    }

    #[test]
    fn signalled_silence_ends_a_turn() {
        let mut speech = speech(Fake(0));
        assert!(speech.feed(&signal(&[(true, 1000)])).is_none());
        assert!(speech.silence(Duration::from_secs(3600)).is_some());
    }

    // ARGOS_SPEECH_MODEL=/path/to/model.gguf cargo test --release speech::tests::measure -- --ignored --nocapture
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "needs ARGOS_SPEECH_MODEL and the macOS say and afconvert tools"]
    fn measure() {
        use std::{process::Command, thread::sleep};

        let model = std::env::var("ARGOS_SPEECH_MODEL").expect("set ARGOS_SPEECH_MODEL");
        let loading = Instant::now();
        let mut speech = Speech::open(Path::new(&model)).unwrap();
        println!("model load: {:?}", loading.elapsed());

        let dir = std::env::temp_dir();
        let (aiff, wav) = (
            dir.join("argos-measure.aiff"),
            dir.join("argos-measure.wav"),
        );
        for expected in [
            "Yes, that works for me.",
            "Could you walk me through how the billing migration handles customers who \
             are still on the legacy annual plan, and what happens to their invoices?",
        ] {
            assert!(
                Command::new("say")
                    .arg("-o")
                    .arg(&aiff)
                    .arg(expected)
                    .status()
                    .unwrap()
                    .success()
            );
            assert!(
                Command::new("afconvert")
                    .args(["-f", "WAVE", "-d", "LEF32@48000", "-c", "1"])
                    .arg(&aiff)
                    .arg(&wav)
                    .status()
                    .unwrap()
                    .success()
            );
            let bytes = std::fs::read(&wav).unwrap();
            let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
            let mut pcm: Vec<f32> = bytes[data..]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            pcm.truncate(pcm.iter().rposition(|s| s.abs() > 0.01).unwrap() + 1);
            let spoken = Duration::from_secs_f64(pcm.len() as f64 / 48000.0);
            pcm.extend(std::iter::repeat_n(0.0, 96000));

            let started = Instant::now();
            let mut busy = Duration::ZERO;
            let mut slowest = Duration::ZERO;
            let mut turn = None;
            for (i, chunk) in pcm.chunks(480).enumerate() {
                sleep((started + Duration::from_millis(10) * (i as u32 + 1)) - Instant::now());
                let fed = Instant::now();
                turn = speech.feed(chunk);
                busy += fed.elapsed();
                slowest = slowest.max(fed.elapsed());
                if turn.is_some() {
                    break;
                }
            }
            let done = Instant::now();
            let turn = turn.expect("no turn");
            assert!(speech.feed(&[0.0; 48000]).is_none());
            let speech_end = started + spoken;
            println!("expected:   {expected}");
            println!("transcript: {}", turn.text);
            println!(
                "audio {spoken:.2?} | real-time factor {:.3} | slowest feed {slowest:.1?}",
                busy.as_secs_f64() / done.duration_since(started).as_secs_f64()
            );
            println!(
                "end of speech to transcript: {:.0?} | reported speech_ended is {:.0?} after the true end",
                done.duration_since(speech_end),
                turn.speech_ended.saturating_duration_since(speech_end)
            );
        }
        let _ = std::fs::remove_file(aiff);
        let _ = std::fs::remove_file(wav);
    }
}
