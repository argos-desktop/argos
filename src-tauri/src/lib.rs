mod keys;
#[cfg(any(windows, target_os = "macos", test))]
mod media;
#[cfg(any(windows, target_os = "macos"))]
mod pipeline;
mod provider;
#[cfg(any(windows, target_os = "macos", test))]
mod session;
mod speech;

#[cfg(windows)]
#[path = "capture/windows.rs"]
mod capture;

#[cfg(target_os = "macos")]
#[path = "capture/macos.rs"]
mod capture;

#[cfg(any(windows, target_os = "macos"))]
mod desktop {
    use super::{
        capture,
        keys::Keys,
        media,
        pipeline::{self, Assistant, Event},
        provider::{self, Provider},
        session::{Shared, Status},
    };
    use serde::Serialize;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };
    use tauri::{Manager, State, WebviewWindow, ipc::Channel};

    const ABOUT_LIMIT: usize = 20_000;

    #[derive(Default)]
    struct App {
        keys: Keys,
        http: reqwest::Client,
        session: Shared,
        worker: Mutex<Option<JoinHandle<()>>>,
        closing: AtomicBool,
        ask: Arc<AtomicBool>,
    }

    #[derive(Serialize)]
    struct Account {
        saved: bool,
        remembered: bool,
    }

    /// The assistant, its credential store account, and its endpoint and model.
    fn assistant(name: &str) -> Result<(Provider, &'static str, String, String), &'static str> {
        let (provider, account, url, model, model_override) = match name {
            "claude" => (
                Provider::Claude,
                "anthropic",
                provider::CLAUDE_URL,
                provider::CLAUDE_MODEL,
                "ARGOS_CLAUDE_MODEL",
            ),
            "codex" => (
                Provider::Codex,
                "openai",
                provider::CODEX_URL,
                provider::CODEX_MODEL,
                "ARGOS_CODEX_MODEL",
            ),
            _ => return Err("Unknown assistant."),
        };
        let model = std::env::var(model_override).unwrap_or_else(|_| model.into());
        Ok((provider, account, url.into(), model))
    }

    // Async so a credential store prompt cannot block the window.
    #[tauri::command]
    async fn account(app: State<'_, App>, provider: String) -> Result<Account, &'static str> {
        let saved = app.keys.get(assistant(&provider)?.1);
        Ok(Account {
            saved: saved.is_some(),
            remembered: saved.is_some_and(|(_, remembered)| remembered),
        })
    }

    #[tauri::command]
    async fn set_key(
        app: State<'_, App>,
        provider: String,
        key: String,
        remember: bool,
    ) -> Result<Account, &'static str> {
        let (provider, account, url, _) = assistant(&provider)?;
        provider::validate_key(&app.http, &url, provider, &key)
            .await
            .map_err(|e| e.message())?;
        Ok(Account {
            saved: true,
            remembered: app.keys.set(account, key, remember),
        })
    }

    #[tauri::command]
    async fn forget_key(app: State<'_, App>, provider: String) -> Result<(), &'static str> {
        app.keys.forget(assistant(&provider)?.1);
        Ok(())
    }

    #[tauri::command]
    fn start(
        window: WebviewWindow,
        app: State<'_, App>,
        provider: Option<String>,
        about: String,
        events: Channel<Event>,
    ) -> Result<(), &'static str> {
        if app.closing.load(Ordering::Acquire) {
            return Err("Argos is closing.");
        }
        let assistant = match provider.as_deref().map(assistant).transpose()? {
            Some((provider, account, base_url, model)) => Some(Assistant {
                http: app.http.clone(),
                provider,
                key: app.keys.get(account).ok_or("Add an API key first.")?.0,
                model,
                base_url,
                ask: app.ask.clone(),
                instructions: match about.trim() {
                    "" => pipeline::INSTRUCTIONS.into(),
                    about => format!(
                        "{}\n\nThe user's own notes about themselves. Tailor answers to them:\n{}",
                        pipeline::INSTRUCTIONS,
                        about.chars().take(ABOUT_LIMIT).collect::<String>()
                    ),
                },
            }),
            None => None,
        };
        let models = window
            .path()
            .app_local_data_dir()
            .map_err(|_| "The Argos data folder is unavailable.")?;
        let session = app.session.clone();
        let token = session.lock().unwrap().start()?;
        let picker = match capture::pick(&window) {
            Ok(picker) => picker,
            Err(e) => {
                session.lock().unwrap().finish(&token, Some(e));
                return Err(e);
            }
        };
        let worker_token = token.clone();
        let (sink, pcm) = media::Sink::new();
        if let Some(assistant) = assistant {
            let (sink, state) = (sink.clone(), session.clone());
            let (running, failed) = (token.clone(), token.clone());
            app.ask.store(false, Ordering::Release);
            let pipeline = thread::Builder::new()
                .name("argos-pipeline".into())
                .spawn(move || {
                    pipeline::run(
                        pcm,
                        sink,
                        running,
                        assistant,
                        models,
                        move |event| {
                            let _ = events.send(event);
                        },
                        move |reason| state.lock().unwrap().fail(&failed, reason),
                    )
                });
            if pipeline.is_err() {
                let e = "Could not start the assistant.";
                session.lock().unwrap().finish(&token, Some(e));
                return Err(e);
            }
        }
        let worker = thread::Builder::new()
            .name("argos-capture".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    capture::run(picker, &session, &worker_token, &sink)
                }))
                .unwrap_or(Err("Capture stopped unexpectedly."));
                session.lock().unwrap().finish(&worker_token, result.err());
            });
        match worker {
            Ok(handle) => {
                *app.worker.lock().unwrap() = Some(handle);
                Ok(())
            }
            Err(_) => {
                let e = "Could not start the capture worker.";
                app.session.lock().unwrap().finish(&token, Some(e));
                Err(e)
            }
        }
    }

    #[tauri::command]
    fn stop(app: State<'_, App>) {
        app.session.lock().unwrap().stop();
    }

    #[tauri::command]
    fn ask(app: State<'_, App>) {
        app.ask.store(true, Ordering::Release);
    }

    /// Floats the window above other apps without a frame, or restores it.
    #[tauri::command]
    fn overlay(window: WebviewWindow, on: bool) {
        let _ = window.set_decorations(!on);
        let _ = window.set_always_on_top(on);
    }

    #[tauri::command]
    fn status(app: State<'_, App>) -> Status {
        app.session.lock().unwrap().status.clone()
    }

    pub fn run() {
        tauri::Builder::default()
            .manage(App {
                http: reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .read_timeout(Duration::from_secs(30))
                    .build()
                    .expect("build the HTTP client"),
                ..App::default()
            })
            .invoke_handler(tauri::generate_handler![
                start, stop, status, ask, overlay, account, set_key, forget_key
            ])
            .on_window_event(|window, event| {
                let app = window.state::<App>();
                match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        if app.closing.swap(true, Ordering::AcqRel) {
                            api.prevent_close();
                            return;
                        }
                        app.session.lock().unwrap().stop();
                        let worker = app.worker.lock().unwrap().take();
                        if let Some(worker) = worker {
                            api.prevent_close();
                            let window = window.clone();
                            // The picker can dispatch to the UI apartment while being released.
                            thread::spawn(move || {
                                // A picker or stream that never completes must not block exit.
                                let deadline = Instant::now() + Duration::from_secs(5);
                                while !worker.is_finished() && Instant::now() < deadline {
                                    thread::sleep(Duration::from_millis(25));
                                }
                                if !worker.is_finished() || window.destroy().is_err() {
                                    window.app_handle().exit(0);
                                }
                            });
                        }
                    }
                    tauri::WindowEvent::Destroyed => app.session.lock().unwrap().stop(),
                    _ => {}
                }
            })
            .run(tauri::generate_context!())
            .expect("run Argos");
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub use desktop::run;
