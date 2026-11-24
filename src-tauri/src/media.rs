use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

const FRAME_INTERVAL: Duration = Duration::from_secs(1);
const PCM_CHUNKS: usize = 256;
const MAX_EDGE: usize = 1568;

/// Where capture hands media to the suggestion pipeline. Both paths drop data under congestion.
pub struct Sink {
    frame: Mutex<(Option<Instant>, Option<Vec<u8>>)>,
    pcm: SyncSender<Vec<f32>>,
}

impl Sink {
    pub fn new() -> (Arc<Self>, Receiver<Vec<f32>>) {
        let (pcm, chunks) = sync_channel(PCM_CHUNKS);
        let sink = Self {
            frame: Mutex::default(),
            pcm,
        };
        (Arc::new(sink), chunks)
    }

    /// Whether `frame` would keep a frame now. Lets capture skip a costly readback.
    pub fn due(&self) -> bool {
        let last = self.frame.lock().unwrap().0;
        last.is_none_or(|at| at.elapsed() >= FRAME_INTERVAL)
    }

    /// Keeps at most one JPEG per interval from packed or row-padded BGRA.
    pub fn frame(&self, bgra: &[u8], stride: usize, width: usize, height: usize) {
        if !self.due() {
            return;
        }
        self.frame.lock().unwrap().0 = Some(Instant::now());
        if let Some(jpeg) = encode(bgra, stride, width, height) {
            self.frame.lock().unwrap().1 = Some(jpeg);
        }
    }

    /// 48 kHz mono samples.
    pub fn pcm(&self, samples: Vec<f32>) {
        let _ = self.pcm.try_send(samples);
    }

    pub fn latest_jpeg(&self) -> Option<Vec<u8>> {
        self.frame.lock().unwrap().1.clone()
    }
}

/// Box-filters by a whole factor so the long edge fits a provider's native image size.
fn shrink(
    bgra: &[u8],
    stride: usize,
    width: usize,
    height: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    let row = width.checked_mul(4)?;
    if width == 0 || height == 0 || stride < row || bgra.len() < stride * (height - 1) + row {
        return None;
    }
    let k = width.max(height).div_ceil(MAX_EDGE);
    let (w, h) = (width / k, height / k);
    if w == 0 || h == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for dy in 0..k {
                let block = &bgra[(y * k + dy) * stride + x * k * 4..][..k * 4];
                for px in block.chunks_exact(4) {
                    sum[0] += u32::from(px[0]);
                    sum[1] += u32::from(px[1]);
                    sum[2] += u32::from(px[2]);
                }
            }
            let n = (k * k) as u32;
            out.extend([
                (sum[0] / n) as u8,
                (sum[1] / n) as u8,
                (sum[2] / n) as u8,
                255,
            ]);
        }
    }
    Some((out, w, h))
}

fn encode(bgra: &[u8], stride: usize, width: usize, height: usize) -> Option<Vec<u8>> {
    let (pixels, w, h) = shrink(bgra, stride, width, height)?;
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 80)
        .encode(&pixels, w as u16, h as u16, jpeg_encoder::ColorType::Bgra)
        .ok()?;
    Some(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_rows_encode_once_per_interval_and_bad_sizes_are_ignored() {
        let (sink, _chunks) = Sink::new();
        sink.frame(&[0; 8], 4, 2, 2);
        assert!(sink.latest_jpeg().is_none());

        let (sink, _chunks) = Sink::new();
        let (w, h, stride) = (5, 3, 32);
        let mut bgra = vec![0xEE; stride * (h - 1) + w * 4];
        for y in 0..h {
            for x in 0..w {
                bgra[y * stride + x * 4..][..4].copy_from_slice(&[0, 0, 255, 255]);
            }
        }
        sink.frame(&bgra, stride, w, h);
        let first = sink.latest_jpeg().expect("encoded frame");
        assert_eq!(&first[..2], &[0xFF, 0xD8]);
        assert_eq!(
            first,
            encode(&[0, 0, 255, 255].repeat(w * h), w * 4, w, h).unwrap()
        );
        sink.frame(&vec![0; stride * h], stride, w, h);
        assert_eq!(sink.latest_jpeg().unwrap(), first);
    }

    #[test]
    fn large_frames_are_averaged_down_to_the_edge_limit() {
        let (w, h) = (MAX_EDGE * 2, 2);
        let bgra: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                if i % 2 == 0 {
                    [0, 0, 0, 9]
                } else {
                    [200, 100, 40, 9]
                }
            })
            .collect();
        let (out, ow, oh) = shrink(&bgra, w * 4, w, h).unwrap();
        assert_eq!((ow, oh), (MAX_EDGE, 1));
        assert!(out.chunks_exact(4).all(|px| px == [100, 50, 20, 255]));
    }

    #[test]
    fn pcm_queue_is_bounded() {
        let (sink, chunks) = Sink::new();
        for _ in 0..PCM_CHUNKS + 10 {
            sink.pcm(vec![0.5]);
        }
        assert_eq!(chunks.try_iter().count(), PCM_CHUNKS);
    }
}
