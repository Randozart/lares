//! Build script: emit the legacy DT_HASH section alongside GNU_HASH.
//!
//! Android 5.1's linker cannot resolve symbols through a GNU-hash-only
//! symbol table, so NativeActivity fails to load the library with the
//! default lld output. This link flag adds the old-style hash table.

fn main() {
    println!("cargo:rustc-link-arg=-Wl,--hash-style=both");
}
