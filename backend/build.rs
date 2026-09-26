// `sqlx::migrate!` embeds each migration file it finds, and on stable Rust
// cargo watches only those: a new migration beside them, with no Rust changed,
// left the crate Fresh and the binary without it -- a release that never
// applied its schema change (thirty-second audit, pass 15). Watching the
// directory makes adding, removing or editing one rebuild the crate.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
