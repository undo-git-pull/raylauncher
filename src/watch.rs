//! FSEvents watcher scheduled on the main run loop: no extra thread, and the kernel
//! batches events so an idle raylauncher uses no CPU. Callbacks receive the changed paths.

use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2_foundation::{NSArray, NSString};

type StreamRef = *mut c_void;
type Callback = extern "C" fn(StreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[repr(C)]
struct Context {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn FSEventStreamCreate(
        alloc: *const c_void,
        cb: Callback,
        ctx: *const Context,
        paths: *const c_void,
        since: u64,
        latency: f64,
        flags: u32,
    ) -> StreamRef;
    fn FSEventStreamScheduleWithRunLoop(s: StreamRef, rl: *mut c_void, mode: *const c_void);
    fn FSEventStreamStart(s: StreamRef) -> bool;
    fn FSEventStreamStop(s: StreamRef);
    fn FSEventStreamInvalidate(s: StreamRef);
    fn FSEventStreamRelease(s: StreamRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetMain() -> *mut c_void;
    static kCFRunLoopDefaultMode: *const c_void;
}

const SINCE_NOW: u64 = u64::MAX;
const FLAG_FILE_EVENTS: u32 = 0x10;

pub struct Watcher {
    stream: StreamRef,
    _handler: Box<Box<dyn Fn(Vec<PathBuf>)>>,
}

extern "C" fn trampoline(_s: StreamRef, info: *mut c_void, n: usize, paths: *mut c_void, _f: *const u32, _i: *const u64) {
    // SAFETY: `info` is the boxed handler owned by the live Watcher; `paths` is a C array
    // of `n` C strings because the stream was created without kFSEventStreamCreateFlagUseCFTypes.
    let handler = unsafe { &*(info as *const Box<dyn Fn(Vec<PathBuf>)>) };
    let paths = paths as *const *const c_char;
    let list = (0..n)
        .map(|i| PathBuf::from(unsafe { CStr::from_ptr(*paths.add(i)) }.to_string_lossy().into_owned()))
        .collect();
    handler(list);
}

impl Watcher {
    /// Watch `dirs` recursively. `latency` seconds coalesces bursts such as an editor's
    /// write-rename-chmod save sequence into one callback.
    pub fn new(dirs: &[PathBuf], latency: f64, handler: impl Fn(Vec<PathBuf>) + 'static) -> Option<Watcher> {
        let strs: Vec<Retained<NSString>> = dirs.iter().map(|d| NSString::from_str(&d.to_string_lossy())).collect();
        if strs.is_empty() {
            return None;
        }
        let array = NSArray::from_retained_slice(&strs);
        let handler: Box<Box<dyn Fn(Vec<PathBuf>)>> = Box::new(Box::new(handler));
        let ctx = Context {
            version: 0,
            info: &*handler as *const Box<dyn Fn(Vec<PathBuf>)> as *mut c_void,
            retain: std::ptr::null(),
            release: std::ptr::null(),
            copy_description: std::ptr::null(),
        };
        unsafe {
            let stream = FSEventStreamCreate(
                std::ptr::null(),
                trampoline,
                &ctx,
                Retained::as_ptr(&array) as *const c_void, // NSArray is toll-free bridged to CFArray
                SINCE_NOW,
                latency,
                FLAG_FILE_EVENTS,
            );
            if stream.is_null() {
                return None;
            }
            FSEventStreamScheduleWithRunLoop(stream, CFRunLoopGetMain(), kCFRunLoopDefaultMode);
            FSEventStreamStart(stream);
            Some(Watcher { stream, _handler: handler })
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            FSEventStreamStop(self.stream);
            FSEventStreamInvalidate(self.stream);
            FSEventStreamRelease(self.stream);
        }
    }
}
