use crate::session::{CaptureResult, Shared, Token, run_inputs};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tauri::WebviewWindow;
use wasapi::{
    AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, SampleType, StreamMode,
    WaveFormat,
};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{
            Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCapturePicker,
            GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice},
            Dxgi::IDXGIDevice,
        },
        System::WinRT::{
            Direct3D11::CreateDirect3D11DeviceFromDXGIDevice, RO_INIT_MULTITHREADED, RoInitialize,
            RoUninitialize,
        },
        UI::Shell::IInitializeWithWindow,
    },
    core::{Error, Interface, Result as WinResult},
};
use windows_future::{AsyncStatus, IAsyncOperation};

type Selection = IAsyncOperation<GraphicsCaptureItem>;

// Called by a synchronous Tauri command on the window's UI thread.
pub fn pick(window: &WebviewWindow) -> CaptureResult<Selection> {
    if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return Err("Windows Graphics Capture is unavailable on this desktop.");
    }
    let picker = GraphicsCapturePicker::new().map_err(|_| "Could not open the Windows picker.")?;
    let owner: IInitializeWithWindow = picker
        .cast()
        .map_err(|_| "Could not attach the Windows picker.")?;
    let hwnd = window
        .hwnd()
        .map_err(|_| "The Argos window is unavailable.")?;
    // The handle belongs to this live WebviewWindow.
    unsafe { owner.Initialize(HWND(hwnd.0)) }
        .map_err(|_| "Could not attach the Windows picker.")?;
    picker
        .PickSingleItemAsync()
        .map_err(|_| "Windows denied or could not start screen selection.")
}

pub fn run(picker: Selection, state: &Shared, token: &Token) -> CaptureResult {
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }
        .map_err(|_| "Windows capture initialization failed.")?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    let _apartment = Apartment;
    // Release the picker operation before leaving the worker's COM apartment.
    let picker = picker;
    let mut cancel_requested = false;
    let item = loop {
        match picker
            .Status()
            .map_err(|_| "Windows picker became unavailable.")?
        {
            AsyncStatus::Started => {
                if token.cancelled() && !cancel_requested {
                    // Cancellation is asynchronous. Keep this generation active until the
                    // picker completes, even if Windows requires the user to dismiss it.
                    let _ = picker.Cancel();
                    cancel_requested = true;
                }
                thread::sleep(Duration::from_millis(25));
            }
            AsyncStatus::Completed => match picker.GetResults() {
                Ok(item) => break item,
                Err(e) if e.code() == Error::empty().code() => return Ok(()),
                Err(_) => return Err("Windows denied screen selection."),
            },
            AsyncStatus::Canceled => return Ok(()),
            _ => return Err("Windows denied or could not complete screen selection."),
        }
    };
    run_inputs(
        state,
        token,
        || Screen::open(item).map_err(|_| "Could not capture the selected source."),
        || Audio::open().map_err(|_| "Could not capture the default playback device."),
        |screen, audio| {
            if let Some(size) = screen.sample()? {
                state.lock().unwrap().frame(token, size.Width, size.Height);
            }
            let (frames, peak) = audio
                .sample()
                .map_err(|_| "Playback device was lost. Check the output device and restart.")?;
            state.lock().unwrap().audio(token, frames, peak);
            Ok(())
        },
    )
}

const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

struct Screen {
    item: GraphicsCaptureItem,
    closed_event: i64,
    closed: Arc<AtomicBool>,
    device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
}
impl Screen {
    fn open(item: GraphicsCaptureItem) -> WinResult<Self> {
        let mut d3d = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
        }
        let dxgi: IDXGIDevice = d3d.ok_or_else(Error::empty)?.cast()?;
        let device: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&device, FORMAT, 1, size)?;
        let session = pool.CreateCaptureSession(&item)?;
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let closed_event = item.Closed(&TypedEventHandler::new(move |_, _| {
            flag.store(true, Ordering::Release);
            Ok(())
        }))?;
        let screen = Self {
            item,
            closed_event,
            closed,
            device,
            pool,
            session,
            size,
        };
        screen.session.StartCapture()?;
        Ok(screen)
    }

    fn sample(&mut self) -> CaptureResult<Option<SizeInt32>> {
        if self.closed.load(Ordering::Acquire) {
            return Err("Selected source closed. Choose a source to restart.");
        }
        let frame = match self.pool.TryGetNextFrame() {
            Ok(frame) => frame,
            Err(e) if e.code() == Error::empty().code() => return Ok(None),
            Err(_) => return Err("Screen capture was interrupted."),
        };
        let size = frame
            .ContentSize()
            .map_err(|_| "Screen capture was interrupted.")?;
        frame
            .Close()
            .map_err(|_| "Screen capture was interrupted.")?;
        drop(frame);
        if size.Width <= 0 || size.Height <= 0 {
            return Ok(None);
        }
        if size != self.size {
            self.pool
                .Recreate(&self.device, FORMAT, 1, size)
                .map_err(|_| "Screen capture could not resize.")?;
            self.size = size;
        }
        Ok(Some(size))
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.item.RemoveClosed(self.closed_event);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

struct Audio {
    client: AudioClient,
    capture: AudioCaptureClient,
    buffer: Vec<u8>,
    peak: f32,
    last_packet: Instant,
}
impl Audio {
    fn open() -> Result<Self, wasapi::WasapiError> {
        let endpoint = DeviceEnumerator::new()?.get_default_device(Direction::Render)?;
        let mut client = endpoint.get_iaudioclient()?;
        let format = WaveFormat::new(32, 32, SampleType::Float, 48_000, 2, None);
        let mode = StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 1_000_000,
        };
        client.initialize_client(&format, Direction::Capture, mode, None)?;
        let capture = client.get_audiocaptureclient()?;
        let buffer = vec![0; client.get_buffer_size()? as usize * format.get_blockalign() as usize];
        client.start_stream()?;
        Ok(Self {
            client,
            capture,
            buffer,
            peak: 0.0,
            last_packet: Instant::now(),
        })
    }
    fn sample(&mut self) -> Result<(u32, f32), wasapi::WasapiError> {
        let mut frames = 0;
        let mut peak = 0.0f32;
        // Bound work per tick even if the endpoint continuously produces packets.
        for _ in 0..16 {
            if self.capture.get_next_packet_size()?.unwrap_or(0) == 0 {
                break;
            }
            let (n, _) = self.capture.read_from_device(&mut self.buffer)?;
            for b in self.buffer[..n as usize * 8].chunks_exact(4) {
                peak = peak.max(f32::from_le_bytes(b.try_into().unwrap()).abs());
            }
            frames += n;
        }
        if frames != 0 {
            self.peak = peak.min(1.0);
            self.last_packet = Instant::now();
        } else if self.last_packet.elapsed() > Duration::from_millis(150) {
            self.peak = 0.0;
        }
        Ok((frames, self.peak))
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}
