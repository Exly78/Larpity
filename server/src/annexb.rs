//! Just enough H.264 Annex B parsing for the fake phone: split a `.h264` file into
//! frames, and read the codec string and picture size from its SPS.

/// One frame (access unit), with its start codes, exactly as a WebCodecs encoder
/// configured with `avc: { format: "annexb" }` would hand it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit<'a> {
    pub data: &'a [u8],
    pub keyframe: bool,
}

const NAL_SLICE: u8 = 1;
const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;

/// Each NAL unit: (where its start code begins, the NAL bytes after the start code).
fn nal_units(stream: &[u8]) -> Vec<(usize, &[u8])> {
    let mut starts = Vec::new(); // (start code begins, NAL begins)
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i..i + 3] == [0, 0, 1] {
            let code_start = if i > 0 && stream[i - 1] == 0 { i - 1 } else { i };
            starts.push((code_start, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &(code_start, nal_start))| {
            let end = starts.get(n + 1).map_or(stream.len(), |next| next.0);
            (code_start, &stream[nal_start..end.max(nal_start)])
        })
        .collect()
}

/// Splits an Annex B stream into frames. A new frame starts at an access unit
/// delimiter, SPS, PPS or SEI, or at a slice whose first macroblock is 0, once
/// the current frame already has a slice.
pub fn access_units(stream: &[u8]) -> Vec<AccessUnit<'_>> {
    let mut units = Vec::new();
    let mut start = None;
    let mut has_slice = false;
    let mut keyframe = false;
    for (offset, nal) in nal_units(stream) {
        let Some(&header) = nal.first() else { continue };
        let kind = header & 0x1f;
        let is_slice = kind == NAL_SLICE || kind == NAL_IDR;
        // first_mb_in_slice is ue(v); a value of 0 is coded as a single 1 bit.
        let first_slice = is_slice && nal.get(1).is_some_and(|b| b & 0x80 != 0);
        let starts_new = has_slice && (first_slice || matches!(kind, 6..=9 | 14..=18));
        if starts_new && let Some(begin) = start {
            units.push(AccessUnit {
                data: &stream[begin..offset],
                keyframe,
            });
            start = None;
            has_slice = false;
            keyframe = false;
        }
        start.get_or_insert(offset);
        has_slice |= is_slice;
        keyframe |= kind == NAL_IDR;
    }
    if let Some(begin) = start
        && has_slice
    {
        units.push(AccessUnit {
            data: &stream[begin..],
            keyframe,
        });
    }
    units
}

/// What the first SPS in a stream says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    /// For WebCodecs, e.g. `avc1.42C01F`.
    pub codec: String,
    pub width: u32,
    pub height: u32,
}

pub fn stream_info(stream: &[u8]) -> Option<StreamInfo> {
    let (_, sps) = nal_units(stream)
        .into_iter()
        .find(|(_, nal)| nal.first().is_some_and(|h| h & 0x1f == NAL_SPS))?;
    let rbsp = unescape(sps.get(1..)?);
    let codec = format!("avc1.{:02X}{:02X}{:02X}", rbsp.first()?, rbsp.get(1)?, rbsp.get(2)?);
    let (width, height) = picture_size(&rbsp)?;
    Some(StreamInfo { codec, width, height })
}

/// Drops the emulation-prevention byte from every `00 00 03`.
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &byte in nal {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// Reads the SPS fields (spec 7.3.2.1.1) up to the cropping window.
fn picture_size(sps: &[u8]) -> Option<(u32, u32)> {
    let mut bits = Bits::new(sps);
    let profile = bits.read(8)?;
    bits.read(16)?; // constraint flags, level
    bits.ue()?; // seq_parameter_set_id
    let mut chroma_format = 1;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format = bits.ue()?;
        if chroma_format == 3 {
            bits.read(1)?; // separate_colour_plane_flag
        }
        bits.ue()?; // bit_depth_luma_minus8
        bits.ue()?; // bit_depth_chroma_minus8
        bits.read(1)?; // qpprime_y_zero_transform_bypass_flag
        if bits.read(1)? == 1 {
            let lists = if chroma_format == 3 { 12 } else { 8 };
            for i in 0..lists {
                if bits.read(1)? == 1 {
                    skip_scaling_list(&mut bits, if i < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    bits.ue()?; // log2_max_frame_num_minus4
    match bits.ue()? {
        0 => {
            bits.ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            bits.read(1)?; // delta_pic_order_always_zero_flag
            bits.se()?; // offset_for_non_ref_pic
            bits.se()?; // offset_for_top_to_bottom_field
            for _ in 0..bits.ue()? {
                bits.se()?;
            }
        }
        _ => {}
    }
    bits.ue()?; // max_num_ref_frames
    bits.read(1)?; // gaps_in_frame_num_value_allowed_flag
    let width_mbs = bits.ue()? + 1;
    let height_map_units = bits.ue()? + 1;
    let frame_mbs_only = bits.read(1)?;
    if frame_mbs_only == 0 {
        bits.read(1)?; // mb_adaptive_frame_field_flag
    }
    bits.read(1)?; // direct_8x8_inference_flag
    let mut width = width_mbs * 16;
    let mut height = (2 - frame_mbs_only) * height_map_units * 16;
    if bits.read(1)? == 1 {
        let (left, right, top, bottom) = (bits.ue()?, bits.ue()?, bits.ue()?, bits.ue()?);
        let (unit_x, unit_y) = match chroma_format {
            0 | 3 => (1, 2 - frame_mbs_only),
            1 => (2, 2 * (2 - frame_mbs_only)),
            _ => (2, 2 - frame_mbs_only),
        };
        width = width.checked_sub(unit_x * (left + right))?;
        height = height.checked_sub(unit_y * (top + bottom))?;
    }
    Some((width, height))
}

fn skip_scaling_list(bits: &mut Bits, size: usize) -> Option<()> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            next = (last + bits.se()? + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, count: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = self.data.get(self.pos / 8)?;
            let bit = (byte >> (7 - self.pos % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.pos += 1;
        }
        Some(value)
    }

    /// Unsigned Exp-Golomb.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.read(zeros)?)
    }

    /// Signed Exp-Golomb.
    fn se(&mut self) -> Option<i64> {
        let k = i64::from(self.ue()?);
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with ffmpeg's libx264; ffprobe gives the frame sizes and keyframes checked below.
    const BASELINE: &[u8] = include_bytes!("../tests/fixtures/baseline-aud-96x64.h264");
    const HIGH_CROPPED: &[u8] = include_bytes!("../tests/fixtures/high-cropped-88x200.h264");

    fn summary(units: &[AccessUnit]) -> Vec<(usize, bool)> {
        units.iter().map(|u| (u.data.len(), u.keyframe)).collect()
    }

    #[test]
    fn splits_frames_at_access_unit_delimiters() {
        let units = access_units(BASELINE);
        assert_eq!(
            summary(&units),
            [(2222, true), (297, false), (353, false), (1757, true), (277, false)]
        );
        assert!(units[0].data.starts_with(&[0, 0, 0, 1]));
    }

    #[test]
    fn splits_frames_without_delimiters() {
        let units = access_units(HIGH_CROPPED);
        assert_eq!(
            summary(&units),
            [(3119, true), (675, false), (2607, true), (633, false)]
        );
    }

    #[test]
    fn frames_cover_the_whole_file() {
        for clip in [BASELINE, HIGH_CROPPED] {
            let total: usize = access_units(clip).iter().map(|u| u.data.len()).sum();
            assert_eq!(total, clip.len());
        }
    }

    #[test]
    fn reads_codec_and_size_from_the_sps() {
        assert_eq!(
            stream_info(BASELINE),
            Some(StreamInfo {
                codec: "avc1.42C00A".into(),
                width: 96,
                height: 64
            })
        );
        // High profile, and a size that isn't a multiple of 16 (so it is cropped).
        assert_eq!(
            stream_info(HIGH_CROPPED),
            Some(StreamInfo {
                codec: "avc1.64000A".into(),
                width: 88,
                height: 200
            })
        );
    }

    #[test]
    fn garbage_gives_nothing() {
        assert!(access_units(b"not h264 at all").is_empty());
        assert!(access_units(&[]).is_empty());
        assert_eq!(stream_info(b"\x00\x00\x01\x67"), None);
    }

    #[test]
    fn emulation_prevention_bytes_are_removed() {
        assert_eq!(unescape(&[1, 0, 0, 3, 1, 0, 0, 3, 0, 0, 3]), [1, 0, 0, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn exp_golomb() {
        // 1 | 010 | 011 | 00100 | 00101 => ue 0, 1, 2, 3; then se of 4 = -2
        let mut bits = Bits::new(&[0b1010_0110, 0b0100_0010, 0b1000_0000]);
        assert_eq!(bits.ue(), Some(0));
        assert_eq!(bits.ue(), Some(1));
        assert_eq!(bits.ue(), Some(2));
        assert_eq!(bits.ue(), Some(3));
        assert_eq!(bits.se(), Some(-2));
    }
}
