fn main() {
    // Version tag for the hand-written scripts in `assets/js` (cache busting).
    println!("cargo:rerun-if-changed=../../assets/js");
    println!("cargo:rerun-if-changed=src");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=WCF_BUILD_ID={stamp:x}");
}
