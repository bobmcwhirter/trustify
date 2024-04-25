use postgresql_embedded::PostgreSQL;
use std::env;
use std::fs::create_dir_all;
use std::process::ExitCode;
use std::time::Duration;
use trustify_common::config::Database;
use trustify_common::db;
use trustify_infrastructure::tracing::{init_tracing, Tracing};

#[derive(clap::Args, Debug)]
pub struct Run {
    #[command(subcommand)]
    pub(crate) command: Command,
    #[command(flatten)]
    pub(crate) database: Database,
}

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    Create,
    Migrate,
    Refresh,
}

impl Run {
    pub async fn run(self) -> anyhow::Result<ExitCode> {
        init_tracing("db-run", Tracing::Disabled);
        use Command::*;
        match self.command {
            Create => self.create().await,
            Migrate => self.migrate().await,
            Refresh => self.refresh().await,
        }
    }

    async fn create(self) -> anyhow::Result<ExitCode> {
        match db::Database::bootstrap(&self.database).await {
            Ok(_) => Ok(ExitCode::SUCCESS),
            Err(e) => Err(e),
        }
    }
    async fn refresh(self) -> anyhow::Result<ExitCode> {
        match db::Database::new(&self.database).await {
            Ok(db) => {
                db.refresh().await?;
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => Err(e),
        }
    }
    async fn migrate(self) -> anyhow::Result<ExitCode> {
        match db::Database::new(&self.database).await {
            Ok(db) => {
                db.migrate().await?;
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<PostgreSQL> {
        init_tracing("db-start", Tracing::Disabled);
        log::warn!("Setting up managed DB; not suitable for production use!");

        let current_dir = env::current_dir()?;
        let work_dir = current_dir.join(".trustify");
        let db_dir = work_dir.join("postgres");
        let data_dir = work_dir.join("data");
        create_dir_all(&data_dir)?;
        let settings = postgresql_embedded::Settings {
            username: self.database.username.clone(),
            password: self.database.password.clone(),
            temporary: false,
            installation_dir: db_dir.clone(),
            timeout: Some(Duration::from_secs(30)),
            data_dir,
            ..Default::default()
        };
        let mut postgresql = PostgreSQL::new(PostgreSQL::default_version(), settings);
        postgresql.setup().await?;
        postgresql.start().await?;

        let port = postgresql.settings().port;
        self.database.port = port;

        log::info!("PostgreSQL installed in {:?}", db_dir);
        log::info!("Running on port {}", port);

        Ok(postgresql)
    }
}
