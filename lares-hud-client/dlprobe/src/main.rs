fn main() {
    let path = std::env::args().nth(1).expect("usage: dlprobe <so-path>");
    unsafe {
        let handle = dlopen(cstr(&path), RTLD_NOW);
        if handle.is_null() {
            let err = dlerror();
            let msg: String = if err.is_null() {
                "unknown".to_string()
            } else {
                std::ffi::CStr::from_ptr(err.cast::<u8>()).to_string_lossy().into_owned()
            };
            println!("FAIL: {}", msg);
        } else {
            println!("OK: loaded");
        }
    }
}

unsafe fn cstr(s: &str) -> *const i8 {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    BOXED.push(bytes);
    BOXED.last().unwrap().as_ptr() as *const i8
}

static mut BOXED: Vec<Vec<u8>> = Vec::new();

const RTLD_NOW: i32 = 2;

extern "C" {
    fn dlopen(file: *const i8, mode: i32) -> *const ();
    fn dlerror() -> *const i8;
}
