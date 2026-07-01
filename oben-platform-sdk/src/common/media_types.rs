/// Helper functions and constants for media type detection.
///
/// Detects audio, video, and image types by examining file extensions
/// or analyzing MIME magic bytes.

/// Known media kind categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Audio,
    Video,
    Image,
    File,
}

/// Known audio extensions.
pub const AUDIO_EXTS: &[&str] = &[".ogg", ".opus", ".mp3", ".wav", ".m4a", ".flac"];

/// Known video extensions.
pub const VIDEO_EXTS: &[&str] = &[".mp4", ".mov", ".webm", ".mkv"];

/// Known image extensions.
pub const IMAGE_EXTS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".svg"];

/// Detect media type from MIME magic bytes.
///
/// # Examples
///
/// ```
/// use oben_platform_sdk::common::media_types::{detect_media_type, MediaKind};
///
/// // PNG magic bytes: 89 50 4E 47
/// let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
/// assert_eq!(detect_media_type(png_bytes), MediaKind::Image);
///
/// // JPEG magic bytes: FF D8 FF
/// let jpeg_bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];
/// assert_eq!(detect_media_type(jpeg_bytes), MediaKind::Image);
///
/// // OGG/Vorbis magic: OggS
/// let ogg_bytes: &[u8] = b'OggS';
/// assert_eq!(detect_media_type(ogg_bytes), MediaKind::Audio);
///
/// // MP4 magic: ftyp
/// let mp4_bytes: &[u8] = b'ftyp';
/// assert_eq!(detect_media_type(mp4_bytes), MediaKind::Video);
/// ```
pub fn detect_media_type(data: &[u8]) -> MediaKind {
    if data.len() < 4 {
        return MediaKind::File;
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data[0] == 0x89 && &data[1..4] == b"PNG" {
        return MediaKind::Image;
    }

    // JPEG: FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return MediaKind::Image;
    }

    // GIF: GIF87a or GIF89a
    if data[0] == b'G' && data[1] == b'I' && data[2] == b'F' {
        return MediaKind::Image;
    }

    // WebP: WEBS
    if data[0] == b'W' && data[1] == b'E' && data[2] == b'B' && data[3] == b'P' {
        return MediaKind::Image;
    }

    // BMP: BM
    if data[0] == b'B' && data[1] == b'M' {
        return MediaKind::Image;
    }

    // TIFF: II or MM
    if (data[0] == b'I' && data[1] == b'I') || (data[0] == b'M' && data[1] == b'M') {
        return MediaKind::Image;
    }

    // OGG: OggS
    if &data[0..4] == b"OggS" {
        return MediaKind::Audio;
    }

    // FLAC: fLaC
    if &data[0..4] == b"fLaC" {
        return MediaKind::Audio;
    }

    // WAV/RIFF-based audio (RIFF header with WAVE)
    if data[0] == b'R' && data[1] == b'I' && data[2] == b'F' && data[3] == b'F' {
        if data.len() >= 12 && &data[8..12] == b"WAVE" {
            return MediaKind::Audio;
        }
    }

    // MP4/MOV: type ftyp after 4-byte length
    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        // Check brand for video or audio
        let brand = &data[8..12];
        return match brand {
            b"isom" | b"mp42" | b"avc1" | b"mmp4" | b"M4V " | b"M4P " => MediaKind::Video,
            b"m4a " | b"m4b " | b"M4B " => MediaKind::Audio,
            _ => MediaKind::Video, // isom/mp42 default to video
        };
    }

    // WebM: 1A 45 DF A3 (EBML header)
    if data[0] == 0x1A && data[1] == 0x45 && data[2] == 0xDF && data[3] == 0xA3 {
        return MediaKind::Video;
    }

    MediaKind::File
}

/// Detect media type from file extension.
///
/// # Examples
///
/// ```
/// use oben_platform_sdk::common::media_types::{ext_to_media, MediaKind};
///
/// assert_eq!(ext_to_media(".mp3"), MediaKind::Audio);
/// assert_eq!(ext_to_media(".mp4"), MediaKind::Video);
/// assert_eq!(ext_to_media(".png"), MediaKind::Image);
/// assert_eq!(ext_to_media(".txt"), MediaKind::File);
/// ```
pub fn ext_to_media(ext: &str) -> MediaKind {
    let ext = ext.to_lowercase();

    if AUDIO_EXTS.iter().any(|&e| ext.ends_with(e)) {
        return MediaKind::Audio;
    }

    if VIDEO_EXTS.iter().any(|&e| ext.ends_with(e)) {
        return MediaKind::Video;
    }

    if IMAGE_EXTS.iter().any(|&e| ext.ends_with(e)) {
        return MediaKind::Image;
    }

    MediaKind::File
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- detect_media_type ---

    /// Given: PNG magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Image
    #[test]
    fn test_detect_media_png() {
        let bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_media_type(bytes), MediaKind::Image);
    }

    /// Given: JPEG magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Image
    #[test]
    fn test_detect_media_jpeg() {
        let bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_media_type(bytes), MediaKind::Image);
    }

    /// Given: GIF magic bytes (GIF89a)
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Image
    #[test]
    fn test_detect_media_gif() {
        let bytes: &[u8] = b"GIF89a";
        assert_eq!(detect_media_type(bytes), MediaKind::Image);
    }

    /// Given: BMP magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Image
    #[test]
    fn test_detect_media_bmp() {
        let bytes: &[u8] = b"BM9999";
        assert_eq!(detect_media_type(bytes), MediaKind::Image);
    }

    /// Given: OGG magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Audio
    #[test]
    fn test_detect_media_ogg() {
        let bytes: &[u8] = b"OggS";
        assert_eq!(detect_media_type(bytes), MediaKind::Audio);
    }

    /// Given: FLAC magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Audio
    #[test]
    fn test_detect_media_flac() {
        let bytes: &[u8] = b"fLaC";
        assert_eq!(detect_media_type(bytes), MediaKind::Audio);
    }

    /// Given: WAV data (RIFF+WAVE header)
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Audio
    #[test]
    fn test_detect_media_wav() {
        let bytes: &[u8] = b"RIFF\x00\x00\x00\x00WAVEfmt ";
        assert_eq!(detect_media_type(bytes), MediaKind::Audio);
    }

    /// Given: MP4 data (ftyp brand)
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Video
    #[test]
    fn test_detect_media_mp4() {
        let bytes: &[u8] = b"\x00\x00\x00\x1cftypisom";
        assert_eq!(detect_media_type(bytes), MediaKind::Video);
    }

    /// Given: M4A audio file (ftyp brand)
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Audio
    #[test]
    fn test_detect_media_m4a() {
        let bytes: &[u8] = b"\x00\x00\x00\x1cftypm4a ";
        assert_eq!(detect_media_type(bytes), MediaKind::Audio);
    }

    /// Given: WebM EBML header
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::Video
    #[test]
    fn test_detect_media_webm() {
        let bytes: &[u8] = &[0x1A, 0x45, 0xDF, 0xA3];
        assert_eq!(detect_media_type(bytes), MediaKind::Video);
    }

    /// Given: Short data (fewer than 4 bytes)
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::File
    #[test]
    fn test_detect_media_short() {
        assert_eq!(detect_media_type(&[0x89, 0x50]), MediaKind::File);
    }

    /// Given: Unknown magic bytes
    /// When: detect_media_type is called
    /// Then: Returns MediaKind::File
    #[test]
    fn test_detect_media_unknown() {
        assert_eq!(detect_media_type(b"Hello, world!"), MediaKind::File);
    }

    // --- ext_to_media ---

    /// Given: Audio extensions
    /// When: ext_to_media is called
    /// Then: Returns MediaKind::Audio for each
    #[test]
    fn test_ext_to_media_audio() {
        for ext in AUDIO_EXTS {
            assert_eq!(ext_to_media(ext), MediaKind::Audio);
        }
    }

    /// Given: Video extensions
    /// When: ext_to_media is called
    /// Then: Returns MediaKind::Video for each
    #[test]
    fn test_ext_to_media_video() {
        for ext in VIDEO_EXTS {
            assert_eq!(ext_to_media(ext), MediaKind::Video);
        }
    }

    /// Given: Image extensions
    /// When: ext_to_media is called
    /// Then: Returns MediaKind::Image for each
    #[test]
    fn test_ext_to_media_image() {
        for ext in IMAGE_EXTS {
            assert_eq!(ext_to_media(ext), MediaKind::Image);
        }
    }

    /// Given: Unknown extension
    /// When: ext_to_media is called
    /// Then: Returns MediaKind::File
    #[test]
    fn test_ext_to_media_unknown() {
        assert_eq!(ext_to_media(".txt"), MediaKind::File);
        assert_eq!(ext_to_media(".json"), MediaKind::File);
    }

    /// Given: Case-insensitive extension matching
    /// When: ext_to_media is called with uppercase extension
    /// Then: Returns the correct MediaKind
    #[test]
    fn test_ext_to_media_case_insensitive() {
        assert_eq!(ext_to_media(".MP3"), MediaKind::Audio);
        assert_eq!(ext_to_media(".MP4"), MediaKind::Video);
        assert_eq!(ext_to_media(".PNG"), MediaKind::Image);
    }

    // --- constants ---

    /// Given: AUDIO_EXTS constant
    /// When: accessed
    /// Then: Contains all expected audio formats
    #[test]
    fn test_audio_exts_nonempty() {
        assert!(!AUDIO_EXTS.is_empty());
        assert!(AUDIO_EXTS.iter().any(|e| *e == ".mp3"));
        assert!(AUDIO_EXTS.iter().any(|e| *e == ".ogg"));
    }

    /// Given: VIDEO_EXTS constant
    /// When: accessed
    /// Then: Contains all expected video formats
    #[test]
    fn test_video_exts_nonempty() {
        assert!(!VIDEO_EXTS.is_empty());
        assert!(VIDEO_EXTS.iter().any(|e| *e == ".mp4"));
        assert!(VIDEO_EXTS.iter().any(|e| *e == ".webm"));
    }

    /// Given: IMAGE_EXTS constant
    /// When: accessed
    /// Then: Contains all expected image formats
    #[test]
    fn test_image_exts_nonempty() {
        assert!(!IMAGE_EXTS.is_empty());
        assert!(IMAGE_EXTS.iter().any(|e| *e == ".jpg"));
        assert!(IMAGE_EXTS.iter().any(|e| *e == ".png"));
    }
}
