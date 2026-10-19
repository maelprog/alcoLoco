//! `sqlx::migrate!` embeds `migrations/` at compile time but does not tell
//! Cargo to watch that directory: without this line, adding or removing a
//! migration file rebuilds nothing, and a warm build keeps serving the
//! previous set of migrations.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
