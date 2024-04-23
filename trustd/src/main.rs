use clap::Parser;
use std::env;
use std::process::{ExitCode, Termination};

mod db;

#[allow(clippy::large_enum_variant)]
#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Run the API server
    Api(trustify_server::Run),
    /// Manage the database
    Db(db::Run),
}

#[derive(clap::Parser, Debug)]
#[command(
    author,
    version = env!("CARGO_PKG_VERSION"),
    about = "trustd",
    long_about = None
)]
pub struct Trustd {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

impl Trustd {
    async fn run(self) -> ExitCode {
        match self.run_command().await {
            Ok(code) => code,
            Err(err) => {
                eprintln!("Error: {err}");
                for (n, err) in err.chain().skip(1).enumerate() {
                    if n == 0 {
                        eprintln!("Caused by:");
                    }
                    eprintln!("\t{err}");
                }

                ExitCode::FAILURE
            }
        }
    }

    async fn run_command(self) -> anyhow::Result<ExitCode> {
        match self.command {
            Some(Command::Api(run)) => run.run().await,
            Some(Command::Db(run)) => run.run().await,
            None => {
                let Some(Command::Db(mut db)) =
                    Trustd::parse_from(["trustd", "db", "migrate"]).command
                else {
                    unreachable!()
                };
                let postgres = db.start().await?;
                if postgres.database_exists(&db.database.name).await.is_err() {
                    db.command = db::Command::Create;
                }
                db.run().await?;

                let api = Trustd::parse_from([
                    "trustd",
                    "api",
                    "--auth-disabled",
                    "--db-port",
                    &postgres.settings().port.to_string(),
                ]);
                Ok(tokio::task::spawn_local(api.run()).await?)
            }
        }
    }
}

#[actix_web::main]
async fn main() -> impl Termination {
    Trustd::parse().run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Trustd::command().debug_assert();
    }
}
