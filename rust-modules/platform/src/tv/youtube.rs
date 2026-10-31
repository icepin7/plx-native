//! Handing a video to the television's own YouTube app: a Seerr title's Trailer (docs/seerr.md).
//! The app plays no YouTube itself; the port asks the platform to open the video in YouTube, off
//! the frame, and says so in a toast when the platform refuses.

/// A YouTube video id as YouTube serves them: letters, digits, `-` and `_`, and nothing that could
/// reach a launch payload as anything but an id.
pub fn is_video_id(video: &str) -> bool {
    !video.is_empty() && video.len() <= 32 && video.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Open `video` in the television's YouTube app. Returns at once: the platform's launch waits on a
/// worker. An id that is not one is refused here, with a log line.
pub fn open(video: &str) {
    if !is_video_id(video) {
        plx_base::eventlog::log("youtube: refused a video id that is not one");
        return;
    }
    (super::port().open_youtube)(video)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_youtube_id_reaches_the_platform() {
        assert!(is_video_id("dQw4w9WgXcQ") && is_video_id("trail-1_A"));
        for bad in ["", "a b", "x\"}", "v=1&y", &"a".repeat(33)] {
            assert!(!is_video_id(bad), "{bad:?}");
        }
    }
}
