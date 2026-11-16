//! Read-only access to the registry: open a key, list subkeys and values, read strings and
//! numbers. Nothing here writes.

// Declared here rather than through windows-sys's Win32_System_Registry feature: one more
// feature makes windows-sys's rustc command line too long for the cmd-based rustc wrapper.
type HKEY = *mut core::ffi::c_void;
pub const HKCU: HKEY = 0x8000_0001u32 as i32 as isize as HKEY;
pub const HKLM: HKEY = 0x8000_0002u32 as i32 as isize as HKEY;
const KEY_READ: u32 = 0x20019;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;
const RRF_RT_REG_SZ: u32 = 0x0002;
const RRF_RT_REG_DWORD: u32 = 0x0010;

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(hkey: HKEY, subkey: *const u16, options: u32, sam: u32, result: *mut HKEY) -> u32;
    fn RegCloseKey(hkey: HKEY) -> u32;
    fn RegEnumKeyExW(hkey: HKEY, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, class: *mut u16, class_len: *mut u32, written: *mut u64) -> u32;
    fn RegEnumValueW(hkey: HKEY, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, kind: *mut u32, data: *mut u8, data_len: *mut u32) -> u32;
    fn RegGetValueW(hkey: HKEY, subkey: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut core::ffi::c_void, data_len: *mut u32) -> u32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Which registry view a 64-bit process reads under HKLM\SOFTWARE.
#[derive(Clone, Copy)]
pub enum View {
    Native,
    Bit32,
}

/// An open key, closed when dropped.
pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

impl Key {
    pub fn open(root: HKEY, path: &str, view: View) -> Option<Key> {
        let flags = KEY_READ | if matches!(view, View::Bit32) { KEY_WOW64_32KEY } else { KEY_WOW64_64KEY };
        let p = wide(path);
        let mut h: HKEY = std::ptr::null_mut();
        let ok = unsafe { RegOpenKeyExW(root, p.as_ptr(), 0, flags, &mut h) } == 0;
        ok.then_some(Key(h))
    }

    pub fn sub(&self, name: &str) -> Option<Key> {
        let p = wide(name);
        let mut h: HKEY = std::ptr::null_mut();
        let ok = unsafe { RegOpenKeyExW(self.0, p.as_ptr(), 0, KEY_READ, &mut h) } == 0;
        ok.then_some(Key(h))
    }

    /// Names of the subkeys (capped, so a damaged hive can't run on forever).
    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u16; 512];
        for i in 0..100_000u32 {
            let mut len = buf.len() as u32;
            let r = unsafe {
                RegEnumKeyExW(self.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
            };
            match r {
                0 => out.push(String::from_utf16_lossy(&buf[..len as usize])),
                234 => continue, // ERROR_MORE_DATA: a name too long to be real, skipped
                _ => break,      // ERROR_NO_MORE_ITEMS or anything else
            }
        }
        out
    }

    /// A string value (`REG_EXPAND_SZ` comes back expanded). Empty strings count as missing.
    pub fn string(&self, name: &str) -> Option<String> {
        let n = wide(name);
        let mut size = 0u32;
        unsafe {
            if RegGetValueW(self.0, std::ptr::null(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 {
                return None;
            }
            if size == 0 || size > 1 << 20 {
                return None;
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut size2 = (buf.len() * 2) as u32;
            if RegGetValueW(self.0, std::ptr::null(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size2) != 0 {
                return None;
            }
            let len = (size2 as usize / 2).min(buf.len());
            let s = String::from_utf16_lossy(&buf[..len]);
            let s = s.trim_end_matches('\0').trim().to_string();
            (!s.is_empty()).then_some(s)
        }
    }

    pub fn dword(&self, name: &str) -> Option<u32> {
        let n = wide(name);
        let mut v = 0u32;
        let mut size = 4u32;
        let ok = unsafe { RegGetValueW(self.0, std::ptr::null(), n.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut size) } == 0;
        ok.then_some(v)
    }

    /// Every value as (name, raw bytes). Values over 64 KB are skipped.
    pub fn values(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut name = vec![0u16; 16_384];
        let mut data = vec![0u8; 65_536];
        for i in 0..100_000u32 {
            let mut nlen = name.len() as u32;
            let mut dlen = data.len() as u32;
            let r = unsafe { RegEnumValueW(self.0, i, name.as_mut_ptr(), &mut nlen, std::ptr::null(), std::ptr::null_mut(), data.as_mut_ptr(), &mut dlen) };
            match r {
                0 => out.push((String::from_utf16_lossy(&name[..nlen as usize]), data[..dlen as usize].to_vec())),
                234 => continue,
                _ => break,
            }
        }
        out
    }
}
