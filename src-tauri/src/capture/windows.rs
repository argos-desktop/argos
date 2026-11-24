use crate::{
    media::Sink,
    session::{CaptureResult, Shared, Token, run_inputs},
};
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
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCapturePicker, GraphicsCaptureSession,
        },
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        SizeInt32,
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::IDXGIDevice,
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize,
        },
        UI::Shell::IInitializeWithWindow,
    },
    core::{Error, Interface, Result as WinResult},
};
use windows_future::{AsyncStatus, IAsyncOperation};

// The picker stays alive until its operation completes.
type Selection = (GraphicsCapturePicker, IAsyncOperation<GraphicsCaptureItem>);

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
    let operation = picker
        .PickSingleItemAsync()
        .map_err(|_| "Windows denied or could not start screen selection.")?;
    Ok((picker, operation))
}

pub fn run(picker: Selection, state: &Shared, token: &Token, sink: &Arc<Sink>) -> CaptureResult {
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
    let (_picker, picker) = picker;
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
            if let Some(size) = screen.sample(sink)? {
                state.lock().unwrap().frame(token, size.Width, size.Height);
            }
            let (frames, peak) = audio
                .sample(sink)
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
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
}
impl Screen {
    fn open(item: GraphicsCaptureItem) -> WinResult<Self> {
        let mut d3d = None;
        // Virtual machines and remote sessions may only offer the software adapter.
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let created = unsafe {
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d3d),
                    None,
                    None,
                )
            };
            if created.is_ok() && d3d.is_some() {
                break;
            }
        }
        let d3d: ID3D11Device = d3d.ok_or_else(Error::empty)?;
        let context = unsafe { d3d.GetImmediateContext()? };
        let dxgi: IDXGIDevice = d3d.cast()?;
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
            d3d,
            context,
            pool,
            session,
            size,
        };
        screen.session.StartCapture()?;
        Ok(screen)
    }

    /// Copies the frame to CPU memory. A failed copy only costs this one screenshot.
    fn read(&self, frame: &Direct3D11CaptureFrame, size: SizeInt32, sink: &Sink) -> WinResult<()> {
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe { self.d3d.CreateTexture2D(&desc, None, Some(&mut staging))? };
        let staging = staging.ok_or_else(Error::empty)?;
        // The texture can be larger than the content while a window is being resized.
        let width = (size.Width as usize).min(desc.Width as usize);
        let height = (size.Height as usize).min(desc.Height as usize);
        unsafe {
            self.context.CopyResource(&staging, &texture);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let stride = map.RowPitch as usize;
            // The mapping stays valid for RowPitch × texture height bytes until Unmap.
            let bgra = std::slice::from_raw_parts(
                map.pData as *const u8,
                stride * (height - 1) + width * 4,
            );
            sink.frame(bgra, stride, width, height);
            self.context.Unmap(&staging, 0);
        }
        Ok(())
    }

    fn sample(&mut self, sink: &Sink) -> CaptureResult<Option<SizeInt32>> {
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
        if size.Width > 0 && size.Height > 0 && sink.due() {
            let _ = self.read(&frame, size, sink);
        }
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
            // One second absorbs a slow screenshot on the shared polling loop.
            buffer_duration_hns: 10_000_000,
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
    fn sample(&mut self, sink: &Sink) -> Result<(u32, f32), wasapi::WasapiError> {
        let mut frames = 0;
        let mut peak = 0.0f32;
        // Bound work per tick even if the endpoint continuously produces packets.
        for _ in 0..16 {
            if self.capture.get_next_packet_size()?.unwrap_or(0) == 0 {
                break;
            }
            let (n, _) = self.capture.read_from_device(&mut self.buffer)?;
            let mut mono = Vec::with_capacity(n as usize);
            for pair in self.buffer[..n as usize * 8].chunks_exact(8) {
                let left = f32::from_le_bytes(pair[..4].try_into().unwrap());
                let right = f32::from_le_bytes(pair[4..].try_into().unwrap());
                peak = peak.max(left.abs()).max(right.abs());
                mono.push((left + right) / 2.0);
            }
            sink.pcm(mono);
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
