use std;
use std::ptr::null_mut;
use std::io;
use libc::wcslen;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use winapi::shared::minwindef::{FALSE, DWORD};
use winapi::um::dbghelp::{SYMBOL_INFOW, SymInitializeW, SymCleanup, SymLoadModuleExW, SymFromNameW, SymUnloadModule64 };
use winapi::um::handleapi::{INVALID_HANDLE_VALUE, CloseHandle};
use winapi::um::processthreadsapi::OpenProcess;
use winapi::um::tlhelp32::{CreateToolhelp32Snapshot, TH32CS_SNAPMODULE, MODULEENTRY32W, Module32FirstW, Module32NextW};
use winapi::um::winnt::{HANDLE, PROCESS_VM_READ};

pub type Pid = u32;

#[derive(Debug, Clone, PartialEq)]
pub struct MapRange {
    pub base_addr: usize,
    pub base_size: usize,
    pub pathname: Option<String>
}

impl MapRange  {
    pub fn size(&self) -> usize { self.base_size }
    pub fn start(&self) -> usize { self.base_addr }
    pub fn filename(&self) -> &Option<String> { &self.pathname }
    pub fn is_exec(&self) -> bool { true }
    pub fn is_write(&self) -> bool { true }
    pub fn is_read(&self) -> bool { true }
}

pub fn get_process_maps(pid: Pid) -> io::Result<Vec<MapRange>> {
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, pid);
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let mut module = MODULEENTRY32W{..Default::default()};
        module.dwSize = std::mem::size_of_val(&module) as u32;

        let mut success = Module32FirstW(handle, &mut module);
        if success == 0 {
            CloseHandle(handle);
            return Err(io::Error::last_os_error())
        }

        let mut vec = Vec::new();
        while success != 0 {
            vec.push(MapRange{base_addr: module.modBaseAddr as usize,
                              base_size: module.modBaseSize as usize,
                              pathname: Some(wstr_to_string(module.szExePath.as_ptr()))});

            success = Module32NextW(handle, &mut module);
        }
        CloseHandle(handle);
        Ok(vec)
    }
}

// The rest of this code is utilities for loading windows symbols.
// This uses the dbghelp win32 api to load the symbols for a process,
// since just parsing the PE file with goblin isn't sufficient (symbols
// can be stored in a separate PDB file on Windows)
pub struct SymbolLoader {
    pub process: HANDLE
}

pub struct SymbolModule<'a> {
    pub parent: &'a SymbolLoader,
    pub filename: &'a str,
    pub base: u64
}

impl SymbolLoader {
    pub fn new(pid: Pid) -> io::Result<Self> {
        unsafe {
            let process = OpenProcess(PROCESS_VM_READ, FALSE, pid as DWORD);
            if process == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            if SymInitializeW(process, null_mut(), FALSE) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self{process})
        }
    }

    pub fn address_from_name(&self, name: &str) -> io::Result<(u64, u64)> {
        // Need to allocate extra space for the SYMBOL_INFO structure, otherwise segfaults
        let size =  std::mem::size_of::<SYMBOL_INFOW>() + 256;
        let buffer = vec![0; size];
        let info: * mut SYMBOL_INFOW = buffer.as_ptr() as * mut SYMBOL_INFOW;
        unsafe {
            (*info).SizeOfStruct = size as u32;
            if SymFromNameW(self.process, string_to_wstr(name).as_ptr(), info) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(((*info).ModBase, (*info).Address))
        }
    }

    /// Loads symbols for filename, returns a SymbolModule structure that must be kept alive
    pub fn load_module<'a>(&'a self, filename: &'a str) -> io::Result<SymbolModule<'a>> {
        unsafe {
            let base = SymLoadModuleExW(self.process,
                                        null_mut(),
                                        string_to_wstr(filename).as_ptr(),
                                        null_mut(),
                                        0,
                                        0,
                                        null_mut(),
                                        0);
            if base == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(SymbolModule{parent: self, filename, base})
        }
    }
}

impl Drop for SymbolLoader {
    fn drop(&mut self) {
        unsafe {
            SymCleanup(self.process);
            CloseHandle(self.process);
        }
    }
}

impl<'a> Drop for SymbolModule<'a> {
    fn drop(&mut self) {
        unsafe {
            SymUnloadModule64(self.parent.process, self.base);
        }
    }
}

fn wstr_to_string(ptr: *const u16) -> String {
    let slice = unsafe { std::slice::from_raw_parts(ptr, wcslen(ptr))};
    OsString::from_wide(slice).to_string_lossy().into_owned()
}

pub fn string_to_wstr(val: &str) -> Vec<u16> {
    OsStr::new(val).encode_wide().chain(std::iter::once(0)).collect()
}