//! The operating system's accessibility settings that `"system"` options follow: on Windows,
//! "Animation effects" (`SPI_GETCLIENTAREAANIMATION`) and a contrast theme (`SPI_GETHIGHCONTRAST`);
//! on macOS, Reduce motion and Increase contrast (`NSWorkspace`). Elsewhere both read as off.

/// The system's settings, as last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemPrefs {
    /// The user asked for less movement.
    pub reduce_motion: bool,
    /// The user asked for more contrast.
    pub high_contrast: bool,
}

/// Reads the system's settings now. Anything that can't be read counts as off.
pub fn read() -> SystemPrefs {
    imp::read()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    //! `SystemParametersInfoW`.

    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SystemParametersInfoW,
    };

    use super::SystemPrefs;

    /// See [`super::read`].
    pub fn read() -> SystemPrefs {
        let mut animation: i32 = 1;
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            dwFlags: 0,
            lpszDefaultScheme: std::ptr::null_mut(),
        };
        // SAFETY: each call writes one value of the type its action documents (a BOOL, a
        // HIGHCONTRASTW with `cbSize` set) through a pointer that's valid for the call.
        let (animation_read, contrast_read) = unsafe {
            (
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    (&raw mut animation).cast(),
                    0,
                ),
                SystemParametersInfoW(
                    SPI_GETHIGHCONTRAST,
                    contrast.cbSize,
                    (&raw mut contrast).cast(),
                    0,
                ),
            )
        };
        SystemPrefs {
            reduce_motion: animation_read != 0 && animation == 0,
            high_contrast: contrast_read != 0 && contrast.dwFlags & HCF_HIGHCONTRASTON != 0,
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    //! `NSWorkspace`'s accessibility display options.

    use objc2_app_kit::NSWorkspace;

    use super::SystemPrefs;

    /// See [`super::read`].
    pub fn read() -> SystemPrefs {
        // SAFETY: plain property reads on the shared workspace, which AppKit allows from any
        // thread.
        unsafe {
            let workspace = NSWorkspace::sharedWorkspace();
            SystemPrefs {
                reduce_motion: workspace.accessibilityDisplayShouldReduceMotion(),
                high_contrast: workspace.accessibilityDisplayShouldIncreaseContrast(),
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    //! No system settings are read here.

    use super::SystemPrefs;

    /// See [`super::read`].
    pub fn read() -> SystemPrefs {
        SystemPrefs::default()
    }
}
