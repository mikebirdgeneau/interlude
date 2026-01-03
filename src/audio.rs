use ogg::PacketReader;
use opus::{Channels, Decoder as OpusDecoder};
use rodio::{OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;

const START_OPUS: &[u8] = include_bytes!("../assets/start.opus");
const END_OPUS: &[u8] = include_bytes!("../assets/end.opus");

pub struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
}

impl Audio {
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        Some(Self {
            _stream: stream,
            handle,
        })
    }

    pub fn play_start(&self) {
        play_bytes(&self.handle, START_OPUS);
    }

    pub fn play_end(&self) {
        play_bytes(&self.handle, END_OPUS);
    }
}

fn play_bytes(handle: &OutputStreamHandle, bytes: &'static [u8]) {
    let (samples, channels, sample_rate) = match decode_opus(bytes) {
        Some(decoded) => decoded,
        None => return,
    };
    let source = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
    let sink = match Sink::try_new(handle) {
        Ok(sink) => sink,
        Err(err) => {
            eprintln!("audio sink error: {err}");
            return;
        }
    };
    sink.set_volume(0.5);
    sink.append(source);
    sink.detach();
}

fn decode_opus(bytes: &'static [u8]) -> Option<(Vec<f32>, u16, u32)> {
    let mut reader = PacketReader::new(Cursor::new(bytes));
    let mut decoder: Option<OpusDecoder> = None;
    let mut channels: u16 = 2;
    let sample_rate = 48_000u32;
    let mut samples: Vec<f32> = Vec::new();

    while let Ok(Some(packet)) = reader.read_packet() {
        let data = packet.data;
        if data.starts_with(b"OpusHead") {
            if data.len() >= 10 {
                channels = data[9] as u16;
                let opus_channels = if channels == 1 {
                    Channels::Mono
                } else {
                    Channels::Stereo
                };
                decoder = OpusDecoder::new(sample_rate, opus_channels).ok();
            }
            continue;
        }
        if data.starts_with(b"OpusTags") {
            continue;
        }

        let Some(decoder) = decoder.as_mut() else {
            continue;
        };

        let max_frame = 5760usize;
        let chan_count = channels.max(1) as usize;
        let mut frame = vec![0f32; max_frame * chan_count];
        let decoded = match decoder.decode_float(&data, &mut frame, false) {
            Ok(decoded) => decoded,
            Err(err) => {
                eprintln!("audio decode error: {err}");
                return None;
            }
        };
        let count = decoded * chan_count;
        samples.extend_from_slice(&frame[..count]);
    }

    if samples.is_empty() {
        eprintln!("audio decode error: no samples decoded");
        return None;
    }

    Some((samples, channels, sample_rate))
}
