//! This module only gets compiled if the configured RomFS directory is found and the `romfs`
//! feature is enabled.
//!
//! Configure the path in Cargo.toml (the default path is "romfs"). Paths are relative to the
//! `CARGO_MANIFEST_DIR` environment variable, which is the directory containing the manifest of
//! your package.
//!
//! ```toml
//! [package.metadata.cargo-3ds]
//! romfs_dir = "romfs"
//! ```

use std::ffi::CStr;
use once_cell::sync::Lazy;
use std::sync::Mutex;

use crate::services::ServiceReference;

#[non_exhaustive]
pub struct RomFS {
    _service_handler: ServiceReference,
}

static ROMFS_ACTIVE: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(0));

impl RomFS {
    pub fn init() -> crate::Result<Self> {
        let _service_handler = ServiceReference::new(
            &ROMFS_ACTIVE,
            true,
            || {
                let mount_name = CStr::from_bytes_with_nul(b"romfs\0").unwrap();
                let r = unsafe { ctru_sys::romfsMountSelf(mount_name.as_ptr()) };
                if r < 0 {
                    return Err(r.into());
                }

                Ok(())
            },
            || {
                let mount_name = CStr::from_bytes_with_nul(b"romfs\0").unwrap();
                unsafe { ctru_sys::romfsUnmount(mount_name.as_ptr()) };
            },
        )?;

        Ok(Self { _service_handler })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn romfs_duplicate() {
        let _romfs = RomFS::init().unwrap();
        let value = *ROMFS_ACTIVE.lock().unwrap();

        assert_eq!(value, 1);

        drop(_romfs);

        let value = *ROMFS_ACTIVE.lock().unwrap();

        assert_eq!(value, 0);
    }
}
