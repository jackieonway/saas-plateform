fn main() {
    // sqlx::migrate! embeds the directory at compile time. Track additions as
    // well as edits so a warm library build cannot omit a new migration.
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=ops/identity");
}
