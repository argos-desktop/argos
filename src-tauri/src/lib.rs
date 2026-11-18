#[cfg(any(windows, test))]
mod session;

#[cfg(windows)]
#[path = "capture/windows.rs"]
mod capture;

#[cfg(windows)]
mod desktop {
    use super::{
        capture,
        session::{Shared, Status},
    };
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
    };
    use tauri::{Manager, State, WebviewWindow};

    #[derive(Default)]
    struct App {
        session: Shared,
        worker: Mutex<Option<JoinHandle<()>>>,
        closing: AtomicBool,
    }

    #[tauri::command]
    fn start(window: WebviewWindow, app: State<'_, App>) -> Result<(), &'static str> {
        if app.closing.load(Ordering::Acquire) {
            return Err("Argos is closing.");
        }
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
        let worker = thread::Builder::new()
            .name("argos-capture".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    capture::run(picker, &session, &worker_token)
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
    fn status(app: State<'_, App>) -> Status {
        app.session.lock().unwrap().status.clone()
    }

    pub fn run() {
        tauri::Builder::default()
            .manage(App::default())
            .invoke_handler(tauri::generate_handler![start, stop, status])
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
                                let _ = worker.join();
                                if window.destroy().is_err() {
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

#[cfg(windows)]
pub use desktop::run;
