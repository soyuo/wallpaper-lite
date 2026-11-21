//! Audio-only Media Foundation engine used as the master clock for 1x video playback.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Media::MediaFoundation::{
            CLSID_MFMediaEngineClassFactory, IMFMediaEngine, IMFMediaEngineClassFactory,
            IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_MEDIA_ENGINE_AUDIOONLY,
            MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_EVENT_ERROR,
            MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA, MFCreateAttributes,
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::{BSTR, implement},
};

use crate::MediaError;

const READY: u32 = 1;
const FAILED: u32 = 2;
const MEDIA_TIME_UNITS: f64 = 10_000_000.0;

#[implement(IMFMediaEngineNotify)]
struct EngineNotify {
    state: Arc<AtomicU32>,
    error: Arc<AtomicI32>,
}

impl IMFMediaEngineNotify_Impl for EngineNotify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, param2: u32) -> windows::core::Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.error.store(param2 as i32, Ordering::Release);
            self.state.fetch_or(FAILED, Ordering::Release);
        } else if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 as u32 {
            self.state.fetch_or(READY, Ordering::Release);
        }
        Ok(())
    }
}

pub(super) struct AudioEngine {
    engine: IMFMediaEngine,
    _notify: IMFMediaEngineNotify,
    has_audio: bool,
}

impl AudioEngine {
    pub(super) fn open(path: &Path) -> Result<Self, MediaError> {
        let state = Arc::new(AtomicU32::new(0));
        let error = Arc::new(AtomicI32::new(0));
        let callback: IMFMediaEngineNotify = EngineNotify {
            state: Arc::clone(&state),
            error: Arc::clone(&error),
        }
        .into();
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 1) }.map_err(audio_init_error)?;
        let attributes = attributes.ok_or(MediaError::AudioPlaybackInitializationFailed {
            hresult: 0x80004005u32 as i32,
        })?;
        unsafe {
            attributes
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)
                .map_err(audio_init_error)?;
        }
        let factory: IMFMediaEngineClassFactory = unsafe {
            CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
        }
        .map_err(audio_init_error)?;
        let engine =
            unsafe { factory.CreateInstance(MF_MEDIA_ENGINE_AUDIOONLY.0 as u32, &attributes) }
                .map_err(audio_init_error)?;

        let url = path_to_file_url(path)?;
        unsafe { engine.SetSource(&BSTR::from(url)) }.map_err(audio_init_error)?;

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let event_state = state.load(Ordering::Acquire);
            if event_state & FAILED != 0 {
                return Err(MediaError::AudioPlaybackInitializationFailed {
                    hresult: match error.load(Ordering::Acquire) {
                        0 => 0x80004005u32 as i32,
                        value => value,
                    },
                });
            }
            if event_state & READY != 0 {
                break;
            }
            if Instant::now() >= deadline {
                return Err(MediaError::AudioPlaybackInitializationFailed {
                    hresult: 0x800705B4u32 as i32,
                });
            }
            thread::sleep(Duration::from_millis(10));
        }

        let has_audio = unsafe { engine.HasAudio().as_bool() };
        Ok(Self {
            engine,
            _notify: callback,
            has_audio,
        })
    }

    pub(super) const fn has_audio(&self) -> bool {
        self.has_audio
    }

    pub(super) fn time_100ns(&self) -> Option<i64> {
        if !self.has_audio {
            return None;
        }
        let seconds = unsafe { self.engine.GetCurrentTime() };
        if !seconds.is_finite() || seconds < 0.0 || seconds > i64::MAX as f64 / MEDIA_TIME_UNITS {
            return None;
        }
        Some((seconds * MEDIA_TIME_UNITS).round() as i64)
    }

    pub(super) fn play(&self) -> Result<(), MediaError> {
        if self.has_audio {
            unsafe { self.engine.Play() }.map_err(audio_control_error)?;
        }
        Ok(())
    }

    pub(super) fn pause(&self) -> Result<(), MediaError> {
        if self.has_audio {
            unsafe { self.engine.Pause() }.map_err(audio_control_error)?;
        }
        Ok(())
    }

    pub(super) fn seek(&self, position_100ns: i64) -> Result<(), MediaError> {
        if self.has_audio {
            unsafe {
                self.engine
                    .SetCurrentTime(position_100ns as f64 / MEDIA_TIME_UNITS)
            }
            .map_err(audio_control_error)?;
        }
        Ok(())
    }

    pub(super) fn configure(&self, volume_percent: u8, muted: bool) -> Result<(), MediaError> {
        if self.has_audio {
            unsafe { self.engine.SetVolume(f64::from(volume_percent) / 100.0) }
                .map_err(audio_control_error)?;
            unsafe { self.engine.SetMuted(muted) }.map_err(audio_control_error)?;
        }
        Ok(())
    }

    pub(super) fn volume(&self) -> f64 {
        unsafe { self.engine.GetVolume() }
    }

    pub(super) fn is_muted(&self) -> bool {
        unsafe { self.engine.GetMuted().as_bool() }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = unsafe { self.engine.Shutdown() };
    }
}

fn audio_init_error(error: windows::core::Error) -> MediaError {
    MediaError::AudioPlaybackInitializationFailed {
        hresult: error.code().0,
    }
}

fn audio_control_error(error: windows::core::Error) -> MediaError {
    MediaError::AudioPlaybackControlFailed {
        hresult: error.code().0,
    }
}

fn path_to_file_url(path: &Path) -> Result<String, MediaError> {
    let path = path
        .canonicalize()
        .map_err(|_| MediaError::InvalidVideoPath)?
        .to_string_lossy()
        .replace('\\', "/");
    let (mut url, path) = if let Some(unc_path) = path.strip_prefix("//?/UNC/") {
        (String::from("file://"), unc_path)
    } else if let Some(dos_path) = path.strip_prefix("//?/") {
        (String::from("file:///"), dos_path)
    } else if let Some(unc_path) = path.strip_prefix("//") {
        (String::from("file://"), unc_path)
    } else {
        (String::from("file:///"), path.as_str())
    };
    for byte in path.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~')
        {
            url.push(char::from(*byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(url, "%{byte:02X}");
        }
    }
    Ok(url)
}
