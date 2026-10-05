fn main() {
    // Re-embed migrations whenever the directory changes.
    println!("cargo:rerun-if-changed=../../migrations");
}
