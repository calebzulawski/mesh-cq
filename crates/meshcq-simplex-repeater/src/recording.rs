use ogg::writing::PacketWriteEndInfo;
use std::io::Write;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::format_description::FormatItem;
use time::OffsetDateTime;

const OPUS_FRAME_SAMPLES: usize = 960;

pub fn write_recording(
    recordings_dir: &Path,
    sample_rate_hz: f32,
    samples: &[f32],
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let serial = (now.as_secs() as u32).wrapping_add(now.subsec_nanos());
    let timestamp = match OffsetDateTime::from_unix_timestamp(now.as_secs() as i64) {
        Ok(dt) => dt
            .replace_nanosecond(now.subsec_nanos())
            .ok()
            .and_then(|dt| dt.format(&Rfc3339).ok())
            .unwrap_or_else(|| format!("{}.{}", now.as_secs(), now.subsec_nanos())),
        Err(_) => format!("{}.{}", now.as_secs(), now.subsec_nanos()),
    };
    let filename = format!(
        "msg-{}.ogg",
        format_timestamp_filename(&timestamp, now.as_secs(), now.subsec_nanos())
    );
    let path = recordings_dir.join(filename);
    let file = std::fs::File::create(&path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut ogg = ogg::writing::PacketWriter::new(&mut writer);

    let mut encoder = opus::Encoder::new(
        sample_rate_hz as u32,
        opus::Channels::Mono,
        opus::Application::Audio,
    )?;

    let opus_head = build_opus_head(sample_rate_hz as u32, 1, 0);
    ogg.write_packet(
        opus_head.into_boxed_slice(),
        serial,
        PacketWriteEndInfo::EndPage,
        0,
    )?;
    let opus_tags = build_opus_tags("meshcq-simplex-repeater", &timestamp);
    ogg.write_packet(
        opus_tags.into_boxed_slice(),
        serial,
        PacketWriteEndInfo::EndPage,
        0,
    )?;

    let mut gp: u64 = 0;
    let mut pos = 0usize;
    let mut out = vec![0u8; 4000];
    while pos < samples.len() {
        let remaining = samples.len() - pos;
        let take = remaining.min(OPUS_FRAME_SAMPLES);
        let mut frame = [0f32; OPUS_FRAME_SAMPLES];
        frame[..take].copy_from_slice(&samples[pos..pos + take]);
        let encoded = encoder.encode_float(&frame, &mut out)?;
        pos += take;
        gp = gp.saturating_add(OPUS_FRAME_SAMPLES as u64);
        let is_last = pos >= samples.len();
        let gp_final = if is_last {
            (samples.len() as u64).min(gp)
        } else {
            gp
        };
        let packet_info = if is_last {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        ogg.write_packet(
            out[..encoded].to_vec().into_boxed_slice(),
            serial,
            packet_info,
            gp_final,
        )?;
    }
    writer.flush()?;
    Ok(path)
}

fn build_opus_head(sample_rate_hz: u32, channels: u8, preskip: u16) -> Vec<u8> {
    let mut head = Vec::with_capacity(19);
    head.extend_from_slice(b"OpusHead");
    head.push(1);
    head.push(channels);
    head.extend_from_slice(&preskip.to_le_bytes());
    head.extend_from_slice(&sample_rate_hz.to_le_bytes());
    head.extend_from_slice(&0i16.to_le_bytes());
    head.push(0);
    head
}

fn build_opus_tags(vendor: &str, timestamp: &str) -> Vec<u8> {
    let mut tags = Vec::new();
    tags.extend_from_slice(b"OpusTags");
    tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags.extend_from_slice(vendor.as_bytes());
    tags.extend_from_slice(&1u32.to_le_bytes());
    let comment = format!("TIMESTAMP={}", timestamp);
    tags.extend_from_slice(&(comment.len() as u32).to_le_bytes());
    tags.extend_from_slice(comment.as_bytes());
    tags
}

fn format_timestamp_filename(timestamp: &str, secs: u64, nanos: u32) -> String {
    let format = "[year]-[month]-[day]-[hour][minute][second].[subsecond digits:4]Z";
    let parsed: Result<Vec<FormatItem<'_>>, _> = time::format_description::parse(format);
    if let Ok(items) = parsed {
        if let Ok(dt) = OffsetDateTime::from_unix_timestamp(secs as i64)
            .and_then(|dt| dt.replace_nanosecond(nanos))
        {
            if let Ok(formatted) = dt.format(&items) {
                return formatted;
            }
        }
    }
    timestamp.to_string()
}

pub fn latest_recording_path(recordings_dir: &Path) -> Option<PathBuf> {
    let mut latest: Option<PathBuf> = None;
    for entry in std::fs::read_dir(recordings_dir).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ogg") {
            continue;
        }
        match &latest {
            Some(cur) => {
                if path.file_name()? > cur.file_name()? {
                    latest = Some(path);
                }
            }
            None => latest = Some(path),
        }
    }
    latest
}

pub fn read_recording(
    path: &Path,
    sample_rate_hz: f32,
) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut ogg = ogg::reading::PacketReader::new(&mut reader);
    let mut decoder = opus::Decoder::new(sample_rate_hz as u32, opus::Channels::Mono)?;
    let mut pcm = Vec::new();
    let mut out = vec![0f32; 5760];
    while let Some(packet) = ogg.read_packet()? {
        if packet.data.starts_with(b"OpusHead") || packet.data.starts_with(b"OpusTags") {
            continue;
        }
        let decoded = decoder.decode_float(&packet.data, &mut out, false)?;
        pcm.extend_from_slice(&out[..decoded]);
    }
    Ok(pcm)
}
