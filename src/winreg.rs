//! Small HKCU registry helpers (advapi32), for the Run key and the Windows accent color.

/// Reads a `REG_SZ` value.
pub fn read_string(subkey: &str, value: &str) -> Option<String> {
    let data = sys::get(subkey, value, sys::RRF_RT_REG_SZ)?;
    let wide: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    Some(String::from_utf16_lossy(&wide[..end]))
}

/// Reads a `REG_DWORD` value.
pub fn read_dword(subkey: &str, value: &str) -> Option<u32> {
    let data = sys::get(subkey, value, sys::RRF_RT_REG_DWORD)?;
    Some(u32::from_le_bytes(data.get(..4)?.try_into().ok()?))
}

/// Reads a `REG_BINARY` value.
pub fn read_binary(subkey: &str, value: &str) -> Option<Vec<u8>> {
    sys::get(subkey, value, sys::RRF_RT_REG_BINARY)
}

/// Writes a `REG_SZ` value, creating the key if needed.
pub fn set_string(subkey: &str, value: &str, data: &str) -> Result<(), String> {
    sys::set_string(subkey, value, data)
}

/// Deletes a value; a value that is not there counts as deleted.
pub fn delete_value(subkey: &str, value: &str) -> Result<(), String> {
    sys::delete(subkey, value)
}

/// Blocks until a value directly under `subkey` changes. False if the key cannot be watched.
pub fn wait_for_change(subkey: &str) -> bool {
    sys::wait_for_change(subkey)
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    type Hkey = isize;
    /// `HKEY_CURRENT_USER`: `(HKEY)(ULONG_PTR)(LONG)0x80000001`, sign-extended.
    const HKCU: Hkey = 0x8000_0001_u32 as i32 as isize;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_MORE_DATA: i32 = 234;
    pub const RRF_RT_REG_SZ: u32 = 0x02;
    pub const RRF_RT_REG_BINARY: u32 = 0x08;
    pub const RRF_RT_REG_DWORD: u32 = 0x10;
    const REG_SZ: u32 = 1;
    const KEY_NOTIFY: u32 = 0x0010;
    const REG_NOTIFY_CHANGE_LAST_SET: u32 = 0x04;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: Hkey,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn RegSetKeyValueW(
            key: Hkey,
            subkey: *const u16,
            value: *const u16,
            kind: u32,
            data: *const c_void,
            size: u32,
        ) -> i32;
        fn RegDeleteKeyValueW(key: Hkey, subkey: *const u16, value: *const u16) -> i32;
        fn RegOpenKeyExW(
            key: Hkey,
            subkey: *const u16,
            options: u32,
            access: u32,
            out: *mut Hkey,
        ) -> i32;
        fn RegNotifyChangeKeyValue(
            key: Hkey,
            subtree: i32,
            filter: u32,
            event: isize,
            asynchronous: i32,
        ) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn get(subkey: &str, value: &str, flags: u32) -> Option<Vec<u8>> {
        let (subkey, value) = (wide(subkey), wide(value));
        let mut buf = vec![0u8; 256];
        loop {
            let mut size = buf.len() as u32;
            // SAFETY: NUL-terminated strings; `buf` has `size` writable bytes.
            let status = unsafe {
                RegGetValueW(
                    HKCU,
                    subkey.as_ptr(),
                    value.as_ptr(),
                    flags,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    buf.truncate(size as usize);
                    return Some(buf);
                }
                ERROR_MORE_DATA => buf.resize(size as usize, 0),
                _ => return None,
            }
        }
    }

    pub fn set_string(subkey: &str, value: &str, data: &str) -> Result<(), String> {
        let (subkey, value, data) = (wide(subkey), wide(value), wide(data));
        // SAFETY: NUL-terminated strings; the size includes the terminator.
        let status = unsafe {
            RegSetKeyValueW(
                HKCU,
                subkey.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(status).to_string())
        }
    }

    pub fn delete(subkey: &str, value: &str) -> Result<(), String> {
        let (subkey, value) = (wide(subkey), wide(value));
        // SAFETY: NUL-terminated strings.
        let status = unsafe { RegDeleteKeyValueW(HKCU, subkey.as_ptr(), value.as_ptr()) };
        match status {
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
            e => Err(std::io::Error::from_raw_os_error(e).to_string()),
        }
    }

    pub fn wait_for_change(subkey: &str) -> bool {
        let subkey = wide(subkey);
        let mut key: Hkey = 0;
        // SAFETY: NUL-terminated string; `key` receives an open handle on success.
        if unsafe { RegOpenKeyExW(HKCU, subkey.as_ptr(), 0, KEY_NOTIFY, &mut key) } != ERROR_SUCCESS
        {
            return false;
        }
        // SAFETY: `key` is open; synchronous mode blocks until a value changes.
        let status = unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, 0, 0) };
        // SAFETY: `key` was opened above.
        unsafe { RegCloseKey(key) };
        status == ERROR_SUCCESS
    }
}

#[cfg(not(windows))]
mod sys {
    pub const RRF_RT_REG_SZ: u32 = 0x02;
    pub const RRF_RT_REG_BINARY: u32 = 0x08;
    pub const RRF_RT_REG_DWORD: u32 = 0x10;

    pub fn get(_: &str, _: &str, _: u32) -> Option<Vec<u8>> {
        None
    }
    pub fn set_string(_: &str, _: &str, _: &str) -> Result<(), String> {
        Err("not on Windows".into())
    }
    pub fn delete(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn wait_for_change(_: &str) -> bool {
        false
    }
}
