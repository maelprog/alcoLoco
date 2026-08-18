//! Migration and seeding tool for the local database.
//!
//! ```text
//! cargo run -p db -- migrate   # apply the pending migrations
//! cargo run -p db -- seed      # insert the development data set
//! cargo run -p db -- reset     # drop everything, migrate again, then seed
//! ```
//!
//! The connection string comes from `DATABASE_URL`, defaulting to the
//! credentials of `docker-compose.yml`.

use std::process::ExitCode;

const USAGE: &str = "usage: db <migrate|seed|reset>

  migrate   apply every migration not yet recorded (no-op when up to date)
  seed      insert the development data set into an empty database
  reset     drop the public schema, re-apply every migration, then seed

connection string: $DATABASE_URL, default postgres://alcoloco:alcoloco@localhost:5432/alcoloco";

#[tokio::main]
async fn main() -> ExitCode {
    let Some(command) = std::env::args().nth(1) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(&command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("db: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: &str) -> Result<(), Box<dyn std::error::Error>> {
    if !matches!(command, "migrate" | "seed" | "reset") {
        return Err(format!("unknown command `{command}`\n\n{USAGE}").into());
    }

    let pool = db::connect().await?;
    println!("connected to {}", db::database_url());

    match command {
        "migrate" => {
            db::migrate(&pool).await?;
            println!("migrations applied");
        }
        "seed" => {
            db::seed(&pool).await?;
            println!("development data seeded");
        }
        _ => {
            db::reset(&pool).await?;
            db::seed(&pool).await?;
            println!("database reset and seeded");
        }
    }

    pool.close().await;
    Ok(())
}
