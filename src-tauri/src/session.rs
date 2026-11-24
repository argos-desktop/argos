use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Starting,
    Running,
    Stopping,
}

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub frames: u64,
    pub width: i32,
    pub height: i32,
    pub audio_frames: u64,
    pub peak: f32,
    pub message: Option<&'static str>,
}

#[derive(Clone, Default)]
pub struct Token(Arc<AtomicBool>);
impl Token {
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Default)]
pub struct Session {
    pub status: Status,
    active: Option<Token>,
    failure: Option<&'static str>,
}

impl Session {
    pub fn start(&mut self) -> Result<Token, &'static str> {
        if self.active.is_some() {
            return Err("A session is already active.");
        }
        let token = Token::default();
        self.active = Some(token.clone());
        self.failure = None;
        self.status = Status {
            phase: Phase::Starting,
            ..Status::default()
        };
        Ok(token)
    }
    fn current(&self, token: &Token) -> bool {
        self.active
            .as_ref()
            .is_some_and(|t| Arc::ptr_eq(&t.0, &token.0))
    }
    pub fn stop(&mut self) {
        if let Some(token) = &self.active {
            token.0.store(true, Ordering::Release);
            self.status.phase = Phase::Stopping;
        }
    }
    /// Ends the session with a reason that stays visible, unlike a user's Stop.
    pub fn fail(&mut self, token: &Token, message: &'static str) {
        if self.current(token) && !token.cancelled() {
            self.failure = Some(message);
            self.stop();
        }
    }
    pub fn running(&mut self, token: &Token) -> bool {
        if !self.current(token) || token.cancelled() {
            return false;
        }
        self.status.phase = Phase::Running;
        true
    }
    pub fn frame(&mut self, token: &Token, width: i32, height: i32) {
        if self.current(token) && self.status.phase == Phase::Running {
            self.status.frames += 1;
            self.status.width = width;
            self.status.height = height;
        }
    }
    pub fn audio(&mut self, token: &Token, frames: u32, peak: f32) {
        if self.current(token) && self.status.phase == Phase::Running {
            self.status.audio_frames += u64::from(frames);
            self.status.peak = peak;
        }
    }
    pub fn finish(&mut self, token: &Token, message: Option<&'static str>) {
        if self.current(token) {
            self.status = Status {
                message: self
                    .failure
                    .take()
                    .or(if token.cancelled() { None } else { message }),
                ..Status::default()
            };
            // Ends every worker of this session, including ones that outlive capture.
            token.0.store(true, Ordering::Release);
            self.active = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_start_and_stop_during_picker() {
        let mut s = Session::default();
        let token = s.start().unwrap();
        assert!(s.start().is_err());
        s.stop();
        assert!(token.cancelled());
        assert!(!s.running(&token));
        assert!(s.start().is_err());
        s.finish(&token, None);
        assert!(s.start().is_ok());
    }

    #[test]
    fn cancellation_is_quiet_and_old_callbacks_cannot_change_new_session() {
        let mut s = Session::default();
        let old = s.start().unwrap();
        s.finish(&old, None);
        assert!(s.status.message.is_none());
        let stopped = s.start().unwrap();
        s.stop();
        s.finish(&stopped, Some("Interrupted"));
        assert!(s.status.message.is_none());
        let next = s.start().unwrap();
        assert!(s.running(&next));
        s.frame(&old, 800, 600);
        s.finish(&old, Some("Old error"));
        assert_eq!(s.status.phase, Phase::Running);
        assert_eq!(s.status.frames, 0);
    }

    #[test]
    fn assistant_failure_stops_the_session_and_stays_visible() {
        let mut s = Session::default();
        let token = s.start().unwrap();
        s.fail(&token, "Key rejected");
        assert!(token.cancelled());
        s.fail(&token, "Later noise");
        s.finish(&token, None);
        assert_eq!(s.status.message, Some("Key rejected"));
        let stale = token;
        let next = s.start().unwrap();
        s.fail(&stale, "Old session");
        assert!(!next.cancelled());
    }

    #[test]
    fn startup_failure_survives_poll_until_retry() {
        let mut s = Session::default();
        let token = s.start().unwrap();
        s.finish(&token, Some("Playback unavailable"));
        assert!(token.cancelled());
        assert_eq!(s.status.phase, Phase::Idle);
        assert_eq!(s.status.message, Some("Playback unavailable"));
        s.start().unwrap();
        assert!(s.status.message.is_none());
    }
}

pub type Shared = Arc<std::sync::Mutex<Session>>;
pub type CaptureResult<T = ()> = Result<T, &'static str>;

/// Keep both handles on their owning thread until capture stops or either source fails.
#[cfg(any(windows, test))]
pub fn run_inputs<S, A>(
    state: &Shared,
    token: &Token,
    open_screen: impl FnOnce() -> CaptureResult<S>,
    open_audio: impl FnOnce() -> CaptureResult<A>,
    mut sample: impl FnMut(&mut S, &mut A) -> CaptureResult,
) -> CaptureResult {
    if token.cancelled() {
        return Ok(());
    }
    let mut screen = open_screen()?;
    if token.cancelled() {
        return Ok(());
    }
    let mut audio = open_audio()?;
    if state.lock().unwrap().running(token) {
        while !token.cancelled() {
            sample(&mut screen, &mut audio)?;
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
    Ok(())
}

#[cfg(test)]
mod input_tests {
    use super::*;

    struct Handle(Arc<AtomicBool>);
    impl Drop for Handle {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn failed_audio_start_releases_screen_before_idle() {
        let s = Shared::default();
        let t = s.lock().unwrap().start().unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let result = run_inputs(
            &s,
            &t,
            || Ok(Handle(dropped.clone())),
            || Err::<(), _>("Audio unavailable"),
            |_, _| unreachable!(),
        );
        assert_eq!(result, Err("Audio unavailable"));
        assert!(dropped.load(Ordering::Acquire));
    }

    #[test]
    fn device_loss_releases_both_sources_and_silence_does_not_end_session() {
        let s = Shared::default();
        let t = s.lock().unwrap().start().unwrap();
        let released = [
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        ];
        let mut polls = 0;
        let result = run_inputs(
            &s,
            &t,
            || Ok(Handle(released[0].clone())),
            || Ok(Handle(released[1].clone())),
            |_, _| {
                polls += 1;
                if polls == 1 {
                    let mut session = s.lock().unwrap();
                    session.audio(&t, 0, 0.0);
                    assert_eq!(session.status.phase, Phase::Running);
                    Ok(())
                } else {
                    Err("Device lost")
                }
            },
        );
        assert_eq!(result, Err("Device lost"));
        assert!(released.iter().all(|x| x.load(Ordering::Acquire)));
    }
}
