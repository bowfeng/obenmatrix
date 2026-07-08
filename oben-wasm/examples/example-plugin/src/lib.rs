//! Example WASM plugin — demonstrates guest exporting pure FFI functions.
//!
//! Uses `CString` for all FFI string returns so the host can safely reconstruct
//! and free strings without losing length information (fat pointers can't cross FFI).

use std::ffi::CString;

// ---------------------------------------------------------------------------
// Guest exports — these are called by the host via raw FFI symbols.
// ---------------------------------------------------------------------------

/// Handler type for tool implementations.
pub type HandlerFn = fn(&str) -> String;

/// Handler type for CLI command implementations.
pub type CommandFn = fn(&[String]) -> String;

const TOOLS: &[(&str, HandlerFn)] = &[
    ("hello-world", hello_world_handler),
];

const COMMANDS: &[(&str, CommandFn)] = &[
    ("example-status", example_status_handler),
];

fn hello_world_handler(args: &str) -> String {
    let name = if args.is_empty() {
        "World"
    } else {
        args
    };
    format!("Hello, {}!", name)
}

fn example_status_handler(_args: &[String]) -> String {
    format!(
        "example-plugin v1.0.0\n  tools: hello-world\n  commands: example-status"
    )
}

#[no_mangle]
pub extern "C" fn component_init() {}

#[no_mangle]
pub extern "C" fn plugin_get_tools() -> usize {
    TOOLS.len()
}

#[no_mangle]
pub extern "C" fn plugin_get_tool_name(index: usize) -> *const i8 {
    if index < TOOLS.len() {
        TOOLS[index].0.as_ptr() as *const i8
    } else {
        std::ptr::null()
    }
}

/// Returns a null-terminated owned string via CString.
/// Caller must free via plugin_free_string.
#[no_mangle]
pub extern "C" fn plugin_execute_tool(handler_name: *const i8, args: *const i8) -> *mut c_char {
    let name = unsafe { std::ffi::CStr::from_ptr(handler_name) };
    let args_str = unsafe { std::ffi::CStr::from_ptr(args) };

    let result = TOOLS
        .iter()
        .find(|(n, _)| *n == name.to_str().unwrap_or_default())
        .map(|(_, handler)| handler(args_str.to_str().unwrap_or("")))
        .unwrap_or_else(|| format!("unknown tool: {}", name.to_string_lossy()));

    // CString stores the string + null terminator as raw bytes.
    // When freed, it deallocates the entire allocation.
    let cstr = CString::new(result).unwrap();
    cstr.into_raw() as *mut c_char
}
#[allow(unused_imports)]
use std::os::raw::c_char;

#[no_mangle]
pub extern "C" fn plugin_free_string(s: *const c_char) {
    if !s.is_null() {
        // Reconstruct CString from raw pointer — it owns the allocation.
        unsafe { drop(CString::from_raw(s as *mut c_char)) };
    }
}

#[no_mangle]
pub extern "C" fn plugin_get_commands() -> usize {
    COMMANDS.len()
}

#[no_mangle]
pub extern "C" fn plugin_get_command_name(index: usize) -> *const i8 {
    if index < COMMANDS.len() {
        COMMANDS[index].0.as_ptr() as *const i8
    } else {
        std::ptr::null()
    }
}

#[no_mangle]
pub extern "C" fn plugin_execute_command(
    handler_name: *const i8,
    args: *const *const i8,
    arg_count: usize,
) -> *mut c_char {
    let name = unsafe { std::ffi::CStr::from_ptr(handler_name) };

    let args_vec: Vec<String> = (0..arg_count)
        .map(|i| unsafe {
            std::ffi::CStr::from_ptr(*args.add(i)).to_string_lossy().into_owned()
        })
        .collect();

    let result = COMMANDS
        .iter()
        .find(|(n, _)| *n == name.to_str().unwrap_or_default())
        .map(|(_, handler)| handler(&args_vec))
        .unwrap_or_else(|| format!("unknown command: {}", name.to_string_lossy()));

    let cstr = CString::new(result).unwrap();
    cstr.into_raw() as *mut c_char
}
