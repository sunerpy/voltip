//! `uniffi-bindgen generate --library <libvoltip_rn.so> --language kotlin --out-dir <dir>`: the
//! Kotlin bindings of the phone shell, read from the metadata UniFFI embeds in the library itself.

fn main() {
    uniffi::uniffi_bindgen_main()
}
